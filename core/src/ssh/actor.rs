//! 每服务器一个 actor:独占 russh Handle,管理连接生命周期与全部隧道。
//! 断线检测靠 handler 的 disconnected 回调 + mpsc 关闭(Handle 非 Clone 无法 clone 出来 poll)
use crate::forward::local::{bind_listener, spawn_local_forward};
use crate::forward::remote::{start_remote_forward, stop_remote_forward};
use crate::forward::socks::spawn_socks_forward;
use crate::known_hosts::KnownHosts;
use crate::model::{Forward, ForwardKind, ForwardStatus, Server, ServerStatus};
use crate::secrets::SecretStore;
use crate::ssh::client::{connect, ChannelOpener, Connection, HostKeyDecider, OpenChannelRequest};
use crate::ssh::TunnelEvent;
use crate::CoreError;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc, Mutex};
use tokio::task::JoinHandle;

pub enum ActorCommand {
    Connect,
    Disconnect,
    StartForward(Forward),
    StopForward { forward_id: String },
    SetAutoReconnect(bool),
    Shutdown,
}

#[derive(Clone)]
pub struct ActorHandle {
    tx: mpsc::Sender<ActorCommand>,
}

impl ActorHandle {
    pub fn send(&self, cmd: ActorCommand) -> Result<(), CoreError> {
        self.tx.try_send(cmd).map_err(|e| CoreError::Other(format!("actor 不可用: {e}")))
    }
}

enum ActiveForward {
    Local(JoinHandle<()>),
    Socks(JoinHandle<()>),
    Remote, // 无本地资源;停止靠 cancel_tcpip_forward
}

struct Actor {
    server: Server,
    secrets: Arc<dyn SecretStore>,
    known_hosts: Arc<Mutex<KnownHosts>>,
    decider: HostKeyDecider,
    auto_reconnect: bool,
    events: broadcast::Sender<TunnelEvent>,
    rx: mpsc::Receiver<ActorCommand>,
    open_tx: mpsc::Sender<OpenChannelRequest>,
    open_rx: mpsc::Receiver<OpenChannelRequest>,
    conn: Option<Connection>,
    forwards: HashMap<String, (Forward, ActiveForward)>,
    /// 手动断开后置位,抑制自动重连
    manual_disconnect: bool,
    /// 下一次重连时刻;None 表示无排程。连接成功必须清空,
    /// 否则旧定时器触发会二次连接、替换掉健康连接
    retry_at: Option<tokio::time::Instant>,
    /// 连续重连失败次数(退避指数)
    attempt: u32,
}

const BACKOFF_INIT: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(30);

pub fn spawn_actor(
    server: Server,
    secrets: Arc<dyn SecretStore>,
    known_hosts: Arc<Mutex<KnownHosts>>,
    decider: HostKeyDecider,
    auto_reconnect: bool,
    events: broadcast::Sender<TunnelEvent>,
) -> ActorHandle {
    let (tx, rx) = mpsc::channel(32);
    let (open_tx, open_rx) = mpsc::channel(64);
    let actor = Actor {
        server, secrets, known_hosts, decider, auto_reconnect, events, rx,
        open_tx, open_rx, conn: None, forwards: HashMap::new(), manual_disconnect: false,
        retry_at: None, attempt: 0,
    };
    tokio::spawn(actor.run());
    ActorHandle { tx }
}

impl Actor {
    fn emit_server(&self, status: ServerStatus, error: Option<String>) {
        // 无订阅者时 send 返回 Err,属正常（如 CLI 场景）,忽略
        let _ = self.events.send(TunnelEvent::ServerStatus { server_id: self.server.id.clone(), status, error });
    }

    fn emit_forward(&self, forward_id: &str, status: ForwardStatus, error: Option<String>) {
        let _ = self.events.send(TunnelEvent::ForwardStatus {
            forward_id: forward_id.to_string(),
            server_id: self.server.id.clone(),
            status,
            error,
        });
    }

    async fn run(mut self) {
        loop {
            tokio::select! {
                cmd = self.rx.recv() => {
                    let Some(cmd) = cmd else { break };
                    match cmd {
                        ActorCommand::Shutdown => break,
                        ActorCommand::Connect => {
                            self.manual_disconnect = false;
                            // 成功路径的 attempt/retry_at 复位由 do_connect 统一完成
                            if self.conn.is_none() {
                                if let Err(e) = self.do_connect().await {
                                    // do_connect 已 emit Error;auto_reconnect 时再转入 Reconnecting
                                    if self.auto_reconnect && !self.manual_disconnect {
                                        self.retry_at = Some(self.next_retry());
                                        self.emit_server(ServerStatus::Reconnecting, Some(e.to_string()));
                                    }
                                }
                            }
                        }
                        ActorCommand::Disconnect => {
                            self.manual_disconnect = true;
                            self.retry_at = None;
                            self.teardown_conn().await;
                            self.emit_server(ServerStatus::Disconnected, None);
                        }
                        ActorCommand::StartForward(f) => self.start_forward(f).await,
                        ActorCommand::StopForward { forward_id } => self.stop_forward(&forward_id).await,
                        ActorCommand::SetAutoReconnect(v) => {
                            self.auto_reconnect = v;
                            // 关闭时取消已排程的重试,否则定时器仍会触发重连
                            if !v {
                                self.retry_at = None;
                            }
                        }
                    }
                }
                // local/socks 转发任务按需请求开 direct-tcpip 通道(跨重连存活,故用最新连接服务)
                req = self.open_rx.recv(), if self.conn.is_some() => {
                    let Some(req) = req else { continue };
                    let conn = self.conn.as_ref().unwrap();
                    let r = conn.handle
                        .channel_open_direct_tcpip(req.target_host, req.target_port, "127.0.0.1", 0)
                        .await
                        .map_err(CoreError::from);
                    let _ = req.respond.send(r);
                }
                // 断线检测:handler.disconnected 回调发来原因;handler 随会话销毁时 recv 返回 None
                msg = async {
                    match self.conn.as_mut() {
                        Some(c) => c.disconnect_rx.recv().await,
                        None => std::future::pending().await,
                    }
                }, if self.conn.is_some() => {
                    let reason = msg.unwrap_or_else(|| "连接已关闭".into());
                    tracing::warn!("SSH 连接断开: {reason}");
                    self.teardown_conn().await;
                    if self.auto_reconnect && !self.manual_disconnect {
                        self.emit_server(ServerStatus::Reconnecting, Some(reason));
                        self.retry_at = Some(self.next_retry());
                    } else {
                        self.emit_server(ServerStatus::Disconnected, Some(reason));
                    }
                }
                // 重连定时器:无重连计划时永久 pending
                () = async {
                    match self.retry_at {
                        Some(t) => tokio::time::sleep_until(t).await,
                        None => std::future::pending().await,
                    }
                } => {
                    self.retry_at = None;
                    // 守卫:已有健康连接(如 Reconnecting 期间 StartForward 隐式连上),
                    // 直接跳过——不得二次连接替换它
                    if self.conn.is_some() {
                        continue;
                    }
                    // 成功路径的 attempt/retry_at 复位由 do_connect 统一完成
                    if let Err(e) = self.do_connect().await {
                        self.emit_server(ServerStatus::Reconnecting, Some(e.to_string()));
                        self.retry_at = Some(self.next_retry());
                    }
                }
            }
        }
        // Shutdown:清理一切。注意 dropping JoinHandle 不会取消任务,
        // local/socks 的 listener 必须显式 abort,否则端口一直被占
        self.teardown_conn().await;
        // 退出前必须发出终态事件:快照/前端/托盘全靠事件驱动(actor 是唯一写者,
        // manager 刻意不代清快照),静默退出会让「已连接/运行中」永远挂着,
        // 之后 stop_forward 找不到 actor 直接 Ok(()) 返回,用户再也无法关掉它。
        // drain 期间持有 forwards 的可变借用,无法调 emit_forward,先收集 id 再统一发
        let mut stopped_ids = Vec::new();
        for (id, (_, active)) in self.forwards.drain() {
            match active {
                ActiveForward::Local(task) | ActiveForward::Socks(task) => task.abort(),
                // 服务器侧 -R 监听随连接断开自动释放,无需 cancel
                ActiveForward::Remote => {}
            }
            stopped_ids.push(id);
        }
        for id in stopped_ids {
            self.emit_forward(&id, ForwardStatus::Stopped, None);
        }
        self.emit_server(ServerStatus::Disconnected, None);
    }

    /// 指数退避:1s/2s/4s/8s/16s/32s→封顶 30s
    fn next_retry(&mut self) -> tokio::time::Instant {
        let delay = BACKOFF_INIT * 2u32.saturating_pow(self.attempt.min(5));
        self.attempt += 1;
        tokio::time::Instant::now() + delay.min(BACKOFF_MAX)
    }

    async fn do_connect(&mut self) -> Result<(), CoreError> {
        self.emit_server(ServerStatus::Connecting, None);
        let conn = connect(&self.server, self.secrets.clone(), self.known_hosts.clone(), self.decider.clone()).await;
        match conn {
            Ok(conn) => {
                self.conn = Some(conn);
                // 连接成功统一复位退避并取消已排程的重连定时器——Connect 命令、
                // 重试定时器、StartForward 隐式连接三条路径共享此不变量,
                // 否则 Reconnecting 期间隐式连上后旧定时器仍会触发二次连接
                self.attempt = 0;
                self.retry_at = None;
                self.emit_server(ServerStatus::Connected, None);
                // 重连后恢复远程转发(local/socks 的 listener 一直活着,通道按需开)
                self.restore_remote_forwards().await;
                Ok(())
            }
            Err(e) => {
                self.emit_server(ServerStatus::Error, Some(e.to_string()));
                Err(e)
            }
        }
    }

    async fn teardown_conn(&mut self) {
        if let Some(conn) = self.conn.take() {
            let _ = conn.handle.disconnect(russh::Disconnect::ByApplication, "bye", "").await;
        }
    }

    async fn restore_remote_forwards(&mut self) {
        let Some(conn) = self.conn.as_ref() else { return };
        // 先 iter_mut 遍历写回分配端口并收集事件,循环结束后再 emit
        // (iter_mut 借用 self.forwards 期间无法再借整个 self 调 emit_forward)
        let mut events = Vec::new();
        for (id, (forward, active)) in self.forwards.iter_mut() {
            if !matches!(active, ActiveForward::Remote) {
                continue;
            }
            match start_remote_forward(forward, &conn.handle, &conn.remote_forwards).await {
                Ok(assigned) => {
                    // 服务器可能分配新端口(原 bind_port=0 或原端口被占),写回副本
                    forward.bind_port = assigned as u16;
                    events.push((id.clone(), ForwardStatus::Running, None));
                }
                Err(e) => events.push((id.clone(), ForwardStatus::Error, Some(e.to_string()))),
            }
        }
        for (id, status, error) in events {
            self.emit_forward(&id, status, error);
        }
    }

    async fn start_forward(&mut self, mut forward: Forward) {
        self.emit_forward(&forward.id, ForwardStatus::Starting, None);
        // 联动规则:未连接先连接
        if self.conn.is_none() {
            if let Err(e) = self.do_connect().await {
                self.emit_forward(&forward.id, ForwardStatus::Error, Some(e.to_string()));
                return;
            }
            // 隐式连接成功:attempt/retry_at 已由 do_connect 复位,
            // 已排程的重连定时器随之取消,不会二次连接
        }
        let Some(conn) = self.conn.as_ref() else { return };
        let result: Result<ActiveForward, CoreError> = async {
            match forward.kind {
                ForwardKind::Local => {
                    let listener = bind_listener(&forward.bind_addr, forward.bind_port).await?;
                    let opener = ChannelOpener::new(self.open_tx.clone());
                    Ok(ActiveForward::Local(spawn_local_forward(
                        listener,
                        opener,
                        forward.target_host.clone().unwrap_or_else(|| "127.0.0.1".into()),
                        forward.target_port.ok_or_else(|| CoreError::Other("缺少目标端口".into()))?,
                    )))
                }
                ForwardKind::Dynamic => {
                    let listener = bind_listener(&forward.bind_addr, forward.bind_port).await?;
                    Ok(ActiveForward::Socks(spawn_socks_forward(listener, ChannelOpener::new(self.open_tx.clone()))))
                }
                ForwardKind::Remote => {
                    let assigned = start_remote_forward(&forward, &conn.handle, &conn.remote_forwards).await?;
                    // 写回服务器分配端口,stop/恢复时按正确端口 cancel 与统计
                    forward.bind_port = assigned as u16;
                    Ok(ActiveForward::Remote)
                }
            }
        }
        .await;
        match result {
            Ok(active) => {
                let id = forward.id.clone();
                self.forwards.insert(id.clone(), (forward, active));
                self.emit_forward(&id, ForwardStatus::Running, None);
            }
            Err(e) => self.emit_forward(&forward.id, ForwardStatus::Error, Some(e.to_string())),
        }
    }

    async fn stop_forward(&mut self, forward_id: &str) {
        let Some((forward, active)) = self.forwards.remove(forward_id) else { return };
        match active {
            ActiveForward::Local(task) | ActiveForward::Socks(task) => task.abort(),
            ActiveForward::Remote => {
                if let Some(conn) = self.conn.as_ref() {
                    // stop_remote_forward 内部无条件清理映射,cancel 失败仅 warn
                    let _ = stop_remote_forward(&forward, &conn.handle, &conn.remote_forwards).await;
                }
            }
        }
        self.emit_forward(forward_id, ForwardStatus::Stopped, None);
    }
}
