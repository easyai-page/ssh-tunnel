use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Server {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth: AuthMethod,
}

// 敏感值不在此处:密码/passphrase 走系统钥匙串,私钥内容落盘 keys/ 目录(见 secrets.rs)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuthMethod {
    Password,
    KeyFile { path: String },
    KeyData,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Forward {
    pub id: String,
    pub server_id: String,
    pub name: String,
    pub kind: ForwardKind,
    pub bind_addr: String,
    pub bind_port: u16,
    /// local: 远程目标；remote: 本地目标；dynamic: 无
    pub target_host: Option<String>,
    pub target_port: Option<u16>,
    pub auto_start: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ForwardKind {
    Local,
    Remote,
    Dynamic,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Settings {
    pub auto_reconnect: bool,
    pub minimize_to_tray: bool,
    pub launch_at_login: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            auto_reconnect: true,
            minimize_to_tray: true,
            launch_at_login: false,
        }
    }
}

// 运行时状态，不持久化
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ServerStatus {
    Disconnected,
    Connecting,
    Connected,
    Reconnecting,
    Error,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ForwardStatus {
    Stopped,
    Starting,
    Running,
    Error,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_serde_roundtrip() {
        let s = Server {
            id: "s1".into(),
            name: "测试服务器".into(),
            host: "192.168.1.10".into(),
            port: 22,
            username: "root".into(),
            auth: AuthMethod::KeyFile {
                path: "/home/u/.ssh/id_ed25519".into(),
            },
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: Server = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
        // 认证方式序列化为带 tag 的形式，保证前端可读
        assert!(json.contains(r#""type":"key_file""#));
    }

    #[test]
    fn forward_kind_snake_case() {
        assert_eq!(
            serde_json::to_string(&ForwardKind::Dynamic).unwrap(),
            r#""dynamic""#
        );
    }

    #[test]
    fn settings_default() {
        let s = Settings::default();
        assert!(s.auto_reconnect && s.minimize_to_tray && !s.launch_at_login);
    }
}
