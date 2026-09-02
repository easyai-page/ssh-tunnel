//! CRUD 编排:配置落盘、secrets 清理、actor 生命周期、状态快照维护
use crate::config::{AppConfig, ConfigStore};
use crate::known_hosts::KnownHosts;
use crate::model::{Forward, Server, Settings};
use crate::secrets::{SecretKind, SecretStore};
use crate::ssh::actor::{spawn_actor, ActorCommand, ActorHandle};
use crate::ssh::client::HostKeyDecider;
use crate::ssh::{ForwardStatusEntry, ServerStatusEntry, StatusSnapshot, TunnelEvent};
use crate::CoreError;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex, RwLock};

pub struct SshManager {
    store: ConfigStore,
    config: Arc<RwLock<AppConfig>>,
    secrets: Arc<dyn SecretStore>,
    known_hosts: Arc<Mutex<KnownHosts>>,
    decider: HostKeyDecider,
    events: broadcast::Sender<TunnelEvent>,
    actors: Arc<RwLock<HashMap<String, ActorHandle>>>,
    snapshot: Arc<RwLock<StatusSnapshot>>,
}

impl SshManager {
    pub fn new(
        store: ConfigStore,
        secrets: Arc<dyn SecretStore>,
        known_hosts: Arc<Mutex<KnownHosts>>,
        decider: HostKeyDecider,
    ) -> Result<Self, CoreError> {
        let config = store.load()?;
        let (events, _) = broadcast::channel(256);
        let snapshot = Arc::new(RwLock::new(StatusSnapshot::default()));
        // 快照跟随事件更新,托盘与前端首屏用
        let mut rx = events.subscribe();
        let snap = snapshot.clone();
        tokio::spawn(async move {
            // 与 Tauri 壳事件任务同理(20dceea):Lagged 只是丢帧,状态事件天然幂等,
            // 下一帧会覆盖;while-let 写法会让跟随任务静默退出,快照从此冻结
            loop {
                match rx.recv().await {
                    Ok(ev) => {
                        let mut snap = snap.write().await;
                        match ev {
                            TunnelEvent::ServerStatus {
                                server_id,
                                status,
                                error,
                            } => {
                                snap.servers
                                    .insert(server_id, ServerStatusEntry { status, error });
                            }
                            TunnelEvent::ForwardStatus {
                                forward_id,
                                status,
                                error,
                                ..
                            } => {
                                snap.forwards
                                    .insert(forward_id, ForwardStatusEntry { status, error });
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        Ok(Self {
            store,
            config: Arc::new(RwLock::new(config)),
            secrets,
            known_hosts,
            decider,
            events,
            actors: Arc::new(RwLock::new(HashMap::new())),
            snapshot,
        })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<TunnelEvent> {
        self.events.subscribe()
    }

    /// 暴露钥匙串访问:Tauri 壳的 upsert_server 需要在认证方式变更时
    /// 清理/写入凭据(凭据不属于配置,manager 不代管这部分写路径)
    pub fn secrets(&self) -> &Arc<dyn SecretStore> {
        &self.secrets
    }

    pub async fn snapshot(&self) -> StatusSnapshot {
        self.snapshot.read().await.clone()
    }

    pub async fn list_servers(&self) -> Vec<Server> {
        self.config.read().await.servers.clone()
    }

    pub async fn list_forwards(&self) -> Vec<Forward> {
        self.config.read().await.forwards.clone()
    }

    pub async fn settings(&self) -> Settings {
        self.config.read().await.settings.clone()
    }

    async fn save(&self) -> Result<(), CoreError> {
        self.store.save(&*self.config.read().await)
    }

    async fn ensure_actor(&self, server: &Server) -> ActorHandle {
        let mut actors = self.actors.write().await;
        if let Some(h) = actors.get(&server.id) {
            return h.clone();
        }
        let auto_reconnect = self.config.read().await.settings.auto_reconnect;
        let handle = spawn_actor(
            server.clone(),
            self.secrets.clone(),
            self.known_hosts.clone(),
            self.decider.clone(),
            auto_reconnect,
            self.events.clone(),
        );
        actors.insert(server.id.clone(), handle.clone());
        handle
    }

    pub async fn upsert_server(&self, mut server: Server) -> Result<Server, CoreError> {
        if server.id.is_empty() {
            server.id = uuid::Uuid::new_v4().to_string();
        }
        // 配置变了连接语义就变了:旧 actor 停掉,下次操作时按新配置重建
        if let Some(actor) = self.actors.write().await.remove(&server.id) {
            let _ = actor.send(ActorCommand::Shutdown);
        }
        {
            let mut cfg = self.config.write().await;
            cfg.servers.retain(|s| s.id != server.id);
            cfg.servers.push(server.clone());
        }
        self.save().await?;
        Ok(server)
    }

    pub async fn delete_server(&self, id: &str) -> Result<(), CoreError> {
        if let Some(actor) = self.actors.write().await.remove(id) {
            let _ = actor.send(ActorCommand::Shutdown);
        }
        let removed_forward_ids: Vec<String> = {
            let mut cfg = self.config.write().await;
            if !cfg.servers.iter().any(|s| s.id == id) {
                return Err(CoreError::ServerNotFound(id.to_string()));
            }
            cfg.servers.retain(|s| s.id != id);
            let ids: Vec<String> = cfg
                .forwards
                .iter()
                .filter(|f| f.server_id == id)
                .map(|f| f.id.clone())
                .collect();
            cfg.forwards.retain(|f| f.server_id != id);
            ids
        };
        for kind in [
            SecretKind::Password,
            SecretKind::Key,
            SecretKind::KeyPassphrase,
        ] {
            let _ = self.secrets.delete(id, kind);
        }
        // 级联删除的转发不会再有事件覆盖其快照,必须一并清除,否则托盘/前端渲染幽灵条目
        let mut snap = self.snapshot.write().await;
        snap.servers.remove(id);
        for fid in removed_forward_ids {
            snap.forwards.remove(&fid);
        }
        drop(snap);
        self.save().await
    }

    pub async fn upsert_forward(&self, mut forward: Forward) -> Result<Forward, CoreError> {
        if forward.id.is_empty() {
            forward.id = uuid::Uuid::new_v4().to_string();
        }
        {
            let cfg = self.config.read().await;
            if !cfg.servers.iter().any(|s| s.id == forward.server_id) {
                return Err(CoreError::ServerNotFound(forward.server_id.clone()));
            }
        }
        // 运行中的转发被修改:先停旧的
        self.stop_forward(&forward.id).await.ok();
        {
            let mut cfg = self.config.write().await;
            cfg.forwards.retain(|f| f.id != forward.id);
            cfg.forwards.push(forward.clone());
        }
        self.save().await?;
        Ok(forward)
    }

    pub async fn delete_forward(&self, id: &str) -> Result<(), CoreError> {
        self.stop_forward(id).await.ok();
        {
            let mut cfg = self.config.write().await;
            let before = cfg.forwards.len();
            cfg.forwards.retain(|f| f.id != id);
            if cfg.forwards.len() == before {
                return Err(CoreError::ForwardNotFound(id.to_string()));
            }
        }
        self.snapshot.write().await.forwards.remove(id);
        self.save().await
    }

    async fn forward_or_err(&self, id: &str) -> Result<Forward, CoreError> {
        self.config
            .read()
            .await
            .forwards
            .iter()
            .find(|f| f.id == id)
            .cloned()
            .ok_or_else(|| CoreError::ForwardNotFound(id.to_string()))
    }

    async fn server_or_err(&self, id: &str) -> Result<Server, CoreError> {
        self.config
            .read()
            .await
            .servers
            .iter()
            .find(|s| s.id == id)
            .cloned()
            .ok_or_else(|| CoreError::ServerNotFound(id.to_string()))
    }

    pub async fn start_forward(&self, id: &str) -> Result<(), CoreError> {
        let forward = self.forward_or_err(id).await?;
        let server = self.server_or_err(&forward.server_id).await?;
        let actor = self.ensure_actor(&server).await;
        actor.send(ActorCommand::StartForward(forward))
    }

    pub async fn stop_forward(&self, id: &str) -> Result<(), CoreError> {
        let forward = self.forward_or_err(id).await?;
        let actors = self.actors.read().await;
        if let Some(actor) = actors.get(&forward.server_id) {
            actor.send(ActorCommand::StopForward {
                forward_id: id.to_string(),
            })?;
        }
        Ok(())
    }

    pub async fn connect_server(&self, id: &str) -> Result<(), CoreError> {
        let server = self.server_or_err(id).await?;
        let actor = self.ensure_actor(&server).await;
        actor.send(ActorCommand::Connect)
    }

    pub async fn disconnect_server(&self, id: &str) -> Result<(), CoreError> {
        let server = self.server_or_err(id).await?;
        let actors = self.actors.read().await;
        if let Some(actor) = actors.get(&server.id) {
            actor.send(ActorCommand::Disconnect)?;
        }
        Ok(())
    }

    pub async fn update_settings(&self, settings: Settings) -> Result<(), CoreError> {
        {
            self.config.write().await.settings = settings.clone();
        }
        self.save().await?;
        let actors = self.actors.read().await;
        for actor in actors.values() {
            let _ = actor.send(ActorCommand::SetAutoReconnect(settings.auto_reconnect));
        }
        Ok(())
    }

    pub async fn start_auto_forwards(&self) {
        let ids: Vec<String> = self
            .config
            .read()
            .await
            .forwards
            .iter()
            .filter(|f| f.auto_start)
            .map(|f| f.id.clone())
            .collect();
        for id in ids {
            if let Err(e) = self.start_forward(&id).await {
                tracing::warn!("自动启动转发 {id} 失败: {e}");
            }
        }
    }

    pub async fn shutdown_all(&self) {
        let actors: Vec<ActorHandle> = self.actors.write().await.values().cloned().collect();
        for actor in actors {
            let _ = actor.send(ActorCommand::Shutdown);
        }
    }
}
