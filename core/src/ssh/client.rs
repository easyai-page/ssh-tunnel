//! russh client 封装：认证、host key 校验、断线通知、-R 通道分发
use crate::known_hosts::{HostKeyStatus, KnownHosts};
use crate::model::{AuthMethod, Server};
use crate::secrets::{SecretKind, SecretStore};
use crate::CoreError;
use russh::client::{self, ChannelOpenHandle, Msg, Session};
use russh::keys::{HashAlg, PrivateKey, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh::{Channel, ChannelOpenFailure};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, Mutex, RwLock};

#[derive(Debug, Clone)]
pub struct HostKeyInfo {
    pub host: String,
    pub port: u16,
    pub fingerprint: String,
    pub is_mismatch: bool,
}

/// host key 未被信任时的决策回调：返回 true 表示信任并记录
pub type HostKeyDecider =
    Arc<dyn Fn(HostKeyInfo) -> Pin<Box<dyn Future<Output = bool> + Send>> + Send + Sync>;

#[derive(Debug, Clone)]
pub struct RemoteTarget {
    pub forward_id: String,
    pub target_host: String,
    pub target_port: u16,
}

/// Handle 非 Clone，转发任务通过此通道向 actor 请求开 direct-tcpip 通道
pub struct OpenChannelRequest {
    pub target_host: String,
    pub target_port: u32,
    pub respond: oneshot::Sender<Result<Channel<Msg>, CoreError>>,
}

#[derive(Clone)]
pub struct ChannelOpener {
    tx: mpsc::Sender<OpenChannelRequest>,
}

impl ChannelOpener {
    pub fn new(tx: mpsc::Sender<OpenChannelRequest>) -> Self {
        Self { tx }
    }

    pub async fn open(&self, host: &str, port: u32) -> Result<Channel<Msg>, CoreError> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(OpenChannelRequest { target_host: host.to_string(), target_port: port, respond: tx })
            .await
            .map_err(|_| CoreError::Other("连接已断开".into()))?;
        rx.await.map_err(|_| CoreError::Other("连接已断开".into()))?
    }
}

pub struct ClientHandler {
    host: String,
    port: u16,
    known_hosts: Arc<Mutex<KnownHosts>>,
    decider: HostKeyDecider,
    remote_forwards: Arc<RwLock<HashMap<u32, RemoteTarget>>>,
    disconnect_tx: mpsc::Sender<String>,
}

impl client::Handler for ClientHandler {
    type Error = CoreError;

    async fn check_server_key(&mut self, key: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        let pubkey = key.public_key();
        let fingerprint = pubkey.fingerprint(HashAlg::Sha256).to_string();
        let status = self.known_hosts.lock().await.check(&self.host, self.port, &pubkey)?;
        match status {
            HostKeyStatus::Trusted => Ok(true),
            HostKeyStatus::Unknown | HostKeyStatus::Changed => {
                let trusted = (self.decider)(HostKeyInfo {
                    host: self.host.clone(),
                    port: self.port,
                    fingerprint,
                    is_mismatch: status == HostKeyStatus::Changed,
                })
                .await;
                // 直接返回 Err 而非 Ok(false)：russh 对 Ok(false) 只会给
                // Error::UnknownKey（映射成 CoreError::Ssh），丢失「用户拒绝」语义
                if !trusted {
                    return Err(CoreError::HostKeyRejected);
                }
                self.known_hosts.lock().await.record(&self.host, self.port, &pubkey)?;
                Ok(true)
            }
        }
    }

    // 服务器侧来连接（-R）：按 connected_port 找到本地目标并桥接
    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: Channel<Msg>,
        _connected_address: &str,
        connected_port: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        let target = self.remote_forwards.read().await.get(&connected_port).cloned();
        match target {
            Some(t) => {
                reply.accept().await;
                tokio::spawn(async move {
                    match tokio::net::TcpStream::connect((t.target_host.as_str(), t.target_port)).await {
                        Ok(mut local) => {
                            let mut stream = channel.into_stream();
                            let _ = tokio::io::copy_bidirectional(&mut stream, &mut local).await;
                        }
                        Err(e) => tracing::warn!("-R 本地目标连接失败: {e}"),
                    }
                });
            }
            None => reply.reject(ChannelOpenFailure::AdministrativelyProhibited).await,
        }
        Ok(())
    }

    async fn disconnected(&mut self, reason: client::DisconnectReason<Self::Error>) -> Result<(), Self::Error> {
        let msg = match reason {
            client::DisconnectReason::ReceivedDisconnect(info) => info.message,
            client::DisconnectReason::Error(e) => e.to_string(),
        };
        // 发送失败说明 actor 已放弃此连接，无需处理
        let _ = self.disconnect_tx.send(msg).await;
        Ok(())
    }
}

/// 一条已建立的连接：handle 由 actor 独占；disconnect_rx 收到消息或对端关闭即断线
pub struct Connection {
    pub handle: client::Handle<ClientHandler>,
    pub disconnect_rx: mpsc::Receiver<String>,
    pub remote_forwards: Arc<RwLock<HashMap<u32, RemoteTarget>>>,
}

pub async fn connect(
    server: &Server,
    secrets: Arc<dyn SecretStore>,
    known_hosts: Arc<Mutex<KnownHosts>>,
    decider: HostKeyDecider,
) -> Result<Connection, CoreError> {
    let config = Arc::new(client::Config {
        inactivity_timeout: None,
        keepalive_interval: Some(Duration::from_secs(15)),
        ..Default::default()
    });
    let remote_forwards = Arc::new(RwLock::new(HashMap::new()));
    let (disconnect_tx, disconnect_rx) = mpsc::channel(1);
    let handler = ClientHandler {
        host: server.host.clone(),
        port: server.port,
        known_hosts,
        decider,
        remote_forwards: remote_forwards.clone(),
        disconnect_tx,
    };
    // 连接与认证共享同一个 10s 上限:对端不回认证响应时 authenticate 一样会挂起,
    // 只包住 client::connect 会让 actor 的 select! 命令臂永久卡死
    let handle = tokio::time::timeout(Duration::from_secs(10), async {
        let mut handle = client::connect(config, (server.host.as_str(), server.port), handler).await?;
        authenticate(&mut handle, server, secrets).await?;
        Ok::<_, CoreError>(handle)
    })
    .await
    .map_err(|_| CoreError::Ssh("连接或认证超时(10s)".into()))??;
    Ok(Connection { handle, disconnect_rx, remote_forwards })
}

/// SecretStore 是同步 trait，KeyringStore 底层是阻塞 IO（dbus/Credential Manager），
/// 统一走 spawn_blocking，避免阻塞 tokio worker
async fn secret_get(
    secrets: &Arc<dyn SecretStore>,
    server_id: &str,
    kind: SecretKind,
) -> Result<Option<String>, CoreError> {
    let secrets = secrets.clone();
    let id = server_id.to_string();
    tokio::task::spawn_blocking(move || secrets.get(&id, kind))
        .await
        .map_err(|e| CoreError::Other(e.to_string()))?
}

async fn authenticate(
    handle: &mut client::Handle<ClientHandler>,
    server: &Server,
    secrets: Arc<dyn SecretStore>,
) -> Result<(), CoreError> {
    match &server.auth {
        AuthMethod::Password => {
            let password = secret_get(&secrets, &server.id, SecretKind::Password)
                .await?
                .ok_or_else(|| CoreError::Auth("未保存密码".into()))?;
            let result = handle.authenticate_password(&server.username, password).await?;
            ensure_success(result)
        }
        AuthMethod::KeyFile { path } => {
            let pass = secret_get(&secrets, &server.id, SecretKind::KeyPassphrase).await?;
            let key = russh::keys::load_secret_key(path, pass.as_deref())
                .map_err(|e| CoreError::Key(format!("读取密钥文件 {path} 失败: {e}")))?;
            auth_with_key(handle, &server.username, key).await
        }
        AuthMethod::KeyData => {
            let data = secret_get(&secrets, &server.id, SecretKind::Key)
                .await?
                .ok_or_else(|| CoreError::Auth("未保存密钥内容".into()))?;
            let pass = secret_get(&secrets, &server.id, SecretKind::KeyPassphrase).await?;
            let key = russh::keys::decode_secret_key(&data, pass.as_deref())
                .map_err(|e| CoreError::Key(format!("解析密钥失败: {e}")))?;
            auth_with_key(handle, &server.username, key).await
        }
    }
}

async fn auth_with_key(
    handle: &mut client::Handle<ClientHandler>,
    username: &str,
    key: PrivateKey,
) -> Result<(), CoreError> {
    // RSA 密钥需要服务器支持 sha2；其他类型此值被 russh 忽略
    let hash_alg = handle.best_supported_rsa_hash().await.ok().flatten().flatten();
    let key = PrivateKeyWithHashAlg::new(Arc::new(key), hash_alg);
    let result = handle.authenticate_publickey(username, key).await?;
    ensure_success(result)
}

fn ensure_success(result: russh::client::AuthResult) -> Result<(), CoreError> {
    match result {
        russh::client::AuthResult::Success => Ok(()),
        russh::client::AuthResult::Failure { .. } => Err(CoreError::Auth("用户名或凭据不正确".into())),
    }
}
