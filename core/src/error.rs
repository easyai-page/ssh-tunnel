#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),
    #[error("配置格式错误: {0}")]
    Json(#[from] serde_json::Error),
    // keyring::Error / russh::Error 不实现 PartialEq，统一转 String
    #[error("钥匙串错误: {0}")]
    Keyring(String),
    #[error("SSH 错误: {0}")]
    Ssh(String),
    #[error("密钥解析失败: {0}")]
    Key(String),
    #[error("认证失败: {0}")]
    Auth(String),
    #[error("本地端口 {0} 被占用")]
    PortInUse(u16),
    #[error("host key 与记录不符（可能遭遇中间人攻击）")]
    HostKeyChanged,
    #[error("用户拒绝了 host key")]
    HostKeyRejected,
    #[error("服务器不存在: {0}")]
    ServerNotFound(String),
    #[error("转发不存在: {0}")]
    ForwardNotFound(String),
    #[error("{0}")]
    Other(String),
}

impl From<russh::Error> for CoreError {
    fn from(e: russh::Error) -> Self {
        CoreError::Ssh(e.to_string())
    }
}

impl From<russh::keys::Error> for CoreError {
    fn from(e: russh::keys::Error) -> Self {
        CoreError::Key(e.to_string())
    }
}

impl From<keyring::Error> for CoreError {
    fn from(e: keyring::Error) -> Self {
        CoreError::Keyring(e.to_string())
    }
}
