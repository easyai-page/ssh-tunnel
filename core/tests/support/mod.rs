//! 进程内测试 SSH 服务器：密码/公钥认证 + direct-tcpip 桥接 + tcpip-forward（-R）模拟
// 本模块是 Task 6-11 集成测试共用的基础设施，单个测试二进制不会用到全部条目，
// 故整体豁免 dead_code，避免每个消费方都触发误报
#![allow(dead_code)]
use russh::keys::{decode_secret_key, PublicKey};
use russh::server::{self, Auth, Config, Msg, RunningServerHandle, Server as _, Session};
use russh::{Channel, ChannelOpenFailure};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

/// 已激活的 -R 监听：(address, port) → accept 循环任务。abort 即释放监听端口
type ForwardMap = Arc<Mutex<HashMap<(String, u32), JoinHandle<()>>>>;

pub const TEST_PASSWORD: &str = "test-password-123";

// 一次性生成的 ed25519 密钥（ssh-keygen 现生成，仅测试用，无 passphrase）
pub const TEST_SERVER_HOST_KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW\nQyNTUxOQAAACA+DoeanTBWDxrSxMpB7n99cAYBH+KJWA6k1w3F5kjhVAAAAJg/lSznP5Us\n5wAAAAtzc2gtZWQyNTUxOQAAACA+DoeanTBWDxrSxMpB7n99cAYBH+KJWA6k1w3F5kjhVA\nAAAEBwkxfG4Qcvs76Hgt3QhMo3pA3dpAVPHmmq3IvfR5hhxD4Oh5qdMFYPGtLEykHuf31w\nBgEf4olYDqTXDcXmSOFUAAAAD3NzaC10dW5uZWwtdGVzdAECAwQFBg==\n-----END OPENSSH PRIVATE KEY-----\n";
// 客户端密钥认证测试用（与服务器 host key 同一把即可，反正是测试）
pub const TEST_CLIENT_KEY: &str = TEST_SERVER_HOST_KEY;

#[derive(Clone, Default)]
pub struct TestServerOpts {
    pub password: Option<&'static str>,
    pub accept_keys: Vec<String>, // openssh 格式的授权公钥
}

#[derive(Clone)]
struct TestServer {
    opts: TestServerOpts,
    forwards: ForwardMap,
}

pub struct TestServerHandle {
    pub addr: SocketAddr,
    pub shutdown: RunningServerHandle,
}

pub async fn start_ssh_server(opts: TestServerOpts) -> TestServerHandle {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let key = decode_secret_key(TEST_SERVER_HOST_KEY, None).unwrap();
    let config = Arc::new(Config {
        keys: vec![key],
        auth_rejection_time: Duration::ZERO,
        ..Default::default()
    });
    let mut server = TestServer { opts, forwards: ForwardMap::default() };
    // run_on_socket 返回的 Future 借用 server 与 listener（非 'static），
    // 因此把二者移进 spawn 任务内部再调用；shutdown 句柄经 oneshot 传回
    let (handle_tx, handle_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let running = server.run_on_socket(config, &listener);
        let _ = handle_tx.send(running.handle());
        let _ = running.await;
    });
    let shutdown = handle_rx.await.unwrap();
    TestServerHandle { addr, shutdown }
}

/// 纯 TCP echo 服务，充当 -L/-D 的转发目标和 -R 的本地目标
pub async fn start_tcp_echo() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                let (mut r, mut w) = socket.split();
                let _ = tokio::io::copy(&mut r, &mut w).await;
            });
        }
    });
    addr
}

fn reject() -> Auth {
    Auth::Reject { proceed_with_methods: None, partial_success: false }
}

struct TestHandler {
    opts: TestServerOpts,
    forwards: ForwardMap,
}

impl server::Server for TestServer {
    type Handler = TestHandler;
    fn new_client(&mut self, _peer: Option<SocketAddr>) -> TestHandler {
        TestHandler { opts: self.opts.clone(), forwards: self.forwards.clone() }
    }
}

impl server::Handler for TestHandler {
    type Error = russh::Error;

    async fn auth_password(&mut self, _user: &str, password: &str) -> Result<Auth, Self::Error> {
        Ok(if Some(password) == self.opts.password { Auth::Accept } else { reject() })
    }

    async fn auth_publickey(&mut self, _user: &str, key: &PublicKey) -> Result<Auth, Self::Error> {
        let offered = key.to_openssh().unwrap_or_default();
        Ok(if self.opts.accept_keys.iter().any(|k| k == &offered) { Auth::Accept } else { reject() })
    }

    // 收到 direct-tcpip 就桥接到真实目标（本测试进程内的 echo 端口）
    async fn channel_open_direct_tcpip(
        &mut self,
        channel: Channel<Msg>,
        host_to_connect: &str,
        port_to_connect: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: russh::server::ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        match tokio::net::TcpStream::connect((host_to_connect, port_to_connect as u16)).await {
            Ok(mut target) => {
                reply.accept().await;
                tokio::spawn(async move {
                    let mut stream = channel.into_stream();
                    let _ = tokio::io::copy_bidirectional(&mut stream, &mut target).await;
                });
            }
            Err(_) => reply.reject(ChannelOpenFailure::ConnectFailed).await,
        }
        Ok(())
    }

    // 模拟真实服务器的 -R：接受转发请求，并在服务器侧监听，来连接时回开 forwarded-tcpip
    async fn tcpip_forward(
        &mut self,
        address: &str,
        port: &mut u32,
        session: &mut Session,
    ) -> Result<bool, Self::Error> {
        let bind_port = *port as u16;
        let listener = match TcpListener::bind((address, bind_port)).await {
            Ok(l) => l,
            Err(_) => return Ok(false),
        };
        let assigned = listener.local_addr().unwrap().port();
        *port = assigned as u32;
        let session_handle = session.handle();
        let connected_address = address.to_string();
        let task = tokio::spawn(async move {
            while let Ok((socket, peer)) = listener.accept().await {
                let Ok(channel) = session_handle
                    .channel_open_forwarded_tcpip(
                        connected_address.clone(),
                        assigned as u32,
                        peer.ip().to_string(),
                        peer.port() as u32,
                    )
                    .await
                else {
                    continue;
                };
                tokio::spawn(async move {
                    let mut stream = channel.into_stream();
                    let mut socket = socket;
                    let _ = tokio::io::copy_bidirectional(&mut stream, &mut socket).await;
                });
            }
        });
        // 记录 accept 任务,cancel_tcpip_forward 据此撤销监听
        self.forwards.lock().await.insert((address.to_string(), assigned as u32), task);
        Ok(true)
    }

    // 撤销 -R:abort accept 循环任务即释放服务器侧监听端口
    async fn cancel_tcpip_forward(
        &mut self,
        address: &str,
        port: u32,
        _session: &mut Session,
    ) -> Result<bool, Self::Error> {
        let removed = self.forwards.lock().await.remove(&(address.to_string(), port));
        match removed {
            Some(task) => {
                task.abort();
                Ok(true)
            }
            None => Ok(false),
        }
    }
}
