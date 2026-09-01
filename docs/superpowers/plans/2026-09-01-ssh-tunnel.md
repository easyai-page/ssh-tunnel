# SSH Tunnel 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 构建跨平台 SSH 端口转发桌面工具（Tauri 2 + Vue 3 + russh），支持多服务器、三种转发类型、系统托盘驻留。

**Architecture:** Cargo workspace 两 crate：`core/`（纯 Rust 核心，零 GUI 依赖，可无头测试）+ `src-tauri/`（Tauri 壳）。每台服务器一条 SSH 连接多路复用所有隧道，由 tokio actor 管理；`client::Handle` 非 Clone，actor 独占，转发任务经 `ChannelOpener`（mpsc）向 actor 请求开通道。前端经 Tauri commands/events 通信。

**Tech Stack:** Rust（tokio + russh 0.63 + keyring 3 + serde + thiserror 2）、Tauri 2、Vue 3 + Pinia + Element Plus、Vitest。

**Spec:** `docs/superpowers/specs/2026-09-01-ssh-tunnel-design.md`

## Global Constraints

- russh 固定 `0.63`（API 已按 0.63.1 源码核对，升级需重查 API）
- 敏感值（密码、密钥内容、passphrase）只进系统钥匙串，禁止落盘/进日志/进错误消息
- 一台服务器一条 SSH 连接，禁止一隧道一连接
- 注释解释「为什么」，用中文；代码与命名用英文
- commit message：`<type>: <中文描述>`，type 用 conventional commits
- 外网命令先直连，失败加代理前缀：`https_proxy=http://127.0.0.1:7897 http_proxy=http://127.0.0.1:7897`
- cargo 走 rsproxy 镜像（仓库根 `.cargo/config.toml`，Task 1 创建）
- 每任务结束跑对应验证命令，全绿后 commit
- TDD：先写失败的测试，再实现

## 已核对的 russh 0.63.1 API（实现时以此为准）

- `client::connect(config: Arc<client::Config>, addrs: impl ToSocketAddrs, handler: H) -> Result<Handle<H>, H::Error>`
- `client::Handle` **非 Clone**。方法：`authenticate_password(&mut self, user, pass)`、`authenticate_publickey(&mut self, user, PrivateKeyWithHashAlg)` → `Result<AuthResult, Error>`（`AuthResult::Success | Failure{..}`）；`channel_open_direct_tcpip(&self, host: impl Into<String>, port: u32, orig_addr: impl Into<String>, orig_port: u32) -> Result<Channel<Msg>>`；`tcpip_forward(&self, addr: impl Into<String>, port: u32) -> Result<u32>`；`cancel_tcpip_forward(&self, addr, port)`；`best_supported_rsa_hash(&self) -> Result<Option<Option<HashAlg>>>`；`disconnect(&self, russh::Disconnect::ByApplication, desc, lang)`
- `client::Handler`：`type Error: From<russh::Error> + Send + Debug`；`async fn check_server_key(&mut self, key: &PublicKeyOrCertificate) -> Result<bool, Self::Error>`；`async fn server_channel_open_forwarded_tcpip(&mut self, channel: Channel<Msg>, connected_address: &str, connected_port: u32, originator_address: &str, originator_port: u32, reply: ChannelOpenHandle, session: &mut client::Session)`；`async fn disconnected(&mut self, reason: DisconnectReason<Self::Error>)`
- `ChannelOpenHandle`：`reply.accept().await`（返回 `()`，channel 用回调参数里的那个）、`reply.reject(ChannelOpenFailure::AdministrativelyProhibited).await`
- `Channel<Msg>`：`wait().await -> Option<ChannelMsg>`、`data_bytes(bytes)`、`into_stream()` → `ChannelStream`（实现 AsyncRead+AsyncWrite，可配 `tokio::io::copy_bidirectional`）
- `russh::keys`：`decode_secret_key(pem: &str, pass: Option<&str>)`、`load_secret_key(path: impl AsRef<Path>, pass)` → `Result<PrivateKey>`；`PrivateKeyWithHashAlg::new(Arc<PrivateKey>, Option<HashAlg>)`；`HashAlg::Sha256`；`PublicKey::to_openssh() -> Result<String>`、`fingerprint(HashAlg)` → 可 `format!("{}", ...)`；`check_known_hosts_path(host, port, &pubkey, path) -> Result<bool>`（`Err(keys::Error::KeyChanged{..})` = 变更，`Ok(false)` = 未知）；`learn_known_hosts_path(host, port, &pubkey, path)`
- `server::Server`：`fn new_client(&mut self, peer: Option<SocketAddr>) -> Self::Handler`；`run_on_socket(&mut self, Arc<Config>, &TcpListener) -> RunningServer<impl Future>`（`RunningServer` 本身即 Future，`.handle()` 得 `RunningServerHandle`，`.shutdown(reason)` 优雅关停）
- `server::Config`：`Default` + 字段 `keys: Vec<PrivateKey>`、`auth_rejection_time: Duration`
- `server::Auth`：`Accept` / `Reject { proceed_with_methods: Option<MethodSet>, partial_success: bool }`
- `server::Handler`：`auth_password/auth_publickey -> Result<Auth>`；`channel_open_direct_tcpip(&mut self, channel, host_to_connect, port_to_connect, originator_address, originator_port, reply, session)`；`tcpip_forward(&mut self, address: &str, port: &mut u32, session: &mut Session) -> Result<bool>`
- `server::Session::handle() -> server::Handle`（Clone）；`server::Handle::channel_open_forwarded_tcpip(conn_addr, conn_port: u32, orig_addr, orig_port: u32) -> Result<Channel<Msg>>`

---

### Task 1: Workspace 脚手架 + 数据模型

**Files:**
- Create: `Cargo.toml`（workspace 根）
- Create: `.cargo/config.toml`
- Create: `core/Cargo.toml`
- Create: `core/src/lib.rs`
- Create: `core/src/model.rs`
- Test: `core/src/model.rs` 内 `#[cfg(test)]`

**Interfaces:**
- Produces: `Server { id, name, host, port: u16, username, auth: AuthMethod }`；`AuthMethod::{Password, KeyFile{path}, KeyData}`（serde tag `type`，snake_case）；`Forward { id, server_id, name, kind: ForwardKind, bind_addr, bind_port: u16, target_host: Option<String>, target_port: Option<u16>, auto_start: bool }`；`ForwardKind::{Local, Remote, Dynamic}`；`Settings { auto_reconnect, minimize_to_tray, launch_at_login }`（Default = true/true/false）；`ServerStatus::{Disconnected,Connecting,Connected,Reconnecting,Error}`；`ForwardStatus::{Stopped,Starting,Running,Error}`。全部 `Debug+Clone+Serialize+Deserialize+PartialEq`。

- [ ] **Step 1: 写 workspace 与 crate 骨架**

`Cargo.toml`：
```toml
[workspace]
resolver = "2"
members = ["core"]  # Task 12 会追加 "src-tauri"

[workspace.package]
edition = "2021"
license = "Apache-2.0"
```

`.cargo/config.toml`（rsproxy 镜像，本机 crates.io 直连不可用；CI 上也可用）：
```toml
[source.crates-io]
replace-with = 'rsproxy-sparse'
[source.rsproxy-sparse]
registry = "sparse+https://rsproxy.cn/index/"
[net]
git-fetch-with-cli = true
```

`core/Cargo.toml`：
```toml
[package]
name = "ssh-tunnel-core"
version = "0.1.0"
edition.workspace = true

[dependencies]
russh = "0.63"
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
uuid = { version = "1", features = ["v4", "serde"] }
keyring = "3"
tracing = "0.1"

[dev-dependencies]
tempfile = "3"
```

`core/src/lib.rs`：
```rust
pub mod model;
pub mod error;
pub mod config;
pub mod secrets;
pub mod known_hosts;
pub mod paths;
pub mod socks5;
pub mod ssh;
pub mod forward;

pub use error::CoreError;
pub use model::*;
```
（此步先只建 `model.rs`，其余模块文件用空文件或 `// 后续任务实现` 占位注释创建，保证 lib.rs 编译通过；后续任务逐个填实。）

- [ ] **Step 2: 写失败测试（model serde roundtrip）**

`core/src/model.rs` 尾部：
```rust
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
            auth: AuthMethod::KeyFile { path: "/home/u/.ssh/id_ed25519".into() },
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: Server = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
        // 认证方式序列化为带 tag 的形式，保证前端可读
        assert!(json.contains(r#""type":"key_file""#));
    }

    #[test]
    fn forward_kind_snake_case() {
        assert_eq!(serde_json::to_string(&ForwardKind::Dynamic).unwrap(), r#""dynamic""#);
    }

    #[test]
    fn settings_default() {
        let s = Settings::default();
        assert!(s.auto_reconnect && s.minimize_to_tray && !s.launch_at_login);
    }
}
```

- [ ] **Step 3: 跑测试确认失败**

Run: `cargo test -p ssh-tunnel-core`
Expected: 编译失败（Server 等类型未定义）

- [ ] **Step 4: 实现 model.rs**

```rust
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

// 敏感值（密码/密钥内容/passphrase）不在此处，全部走钥匙串（见 secrets.rs）
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
        Self { auto_reconnect: true, minimize_to_tray: true, launch_at_login: false }
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
```

- [ ] **Step 5: 跑测试确认通过**

Run: `cargo test -p ssh-tunnel-core`
Expected: 3 passed

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat: 搭建 workspace 骨架与数据模型"
```

---

### Task 2: 错误类型 + 配置存储 + 路径解析

**Files:**
- Create: `core/src/error.rs`
- Create: `core/src/config.rs`
- Create: `core/src/paths.rs`
- Test: `core/src/config.rs` 内 `#[cfg(test)]`

**Interfaces:**
- Consumes: `model.rs` 全部类型
- Produces: `CoreError`（各变体见下）；`AppConfig { version: u32, servers: Vec<Server>, forwards: Vec<Forward>, settings: Settings }`（Default: version=1）；`ConfigStore::new(path: PathBuf)`、`load() -> Result<AppConfig, CoreError>`（文件不存在 → Ok(default)）、`save(&self, &AppConfig) -> Result<(), CoreError>`（原子写，Unix 0600）；`paths::config_dir() -> PathBuf`。

- [ ] **Step 1: 写失败测试**

`core/src/config.rs` 尾部：
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    fn sample_config() -> AppConfig {
        AppConfig {
            version: 1,
            servers: vec![Server {
                id: "s1".into(), name: "db".into(), host: "10.0.0.2".into(),
                port: 22, username: "u".into(), auth: AuthMethod::Password,
            }],
            forwards: vec![Forward {
                id: "f1".into(), server_id: "s1".into(), name: "mysql".into(),
                kind: ForwardKind::Local, bind_addr: "127.0.0.1".into(), bind_port: 3306,
                target_host: Some("127.0.0.1".into()), target_port: Some(3306),
                auto_start: true,
            }],
            settings: Settings::default(),
        }
    }

    #[test]
    fn load_missing_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(dir.path().join("config.json"));
        let cfg = store.load().unwrap();
        assert!(cfg.servers.is_empty() && cfg.version == 1);
    }

    #[test]
    fn save_then_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(dir.path().join("config.json"));
        let cfg = sample_config();
        store.save(&cfg).unwrap();
        assert_eq!(store.load().unwrap(), cfg);
    }

    #[cfg(unix)]
    #[test]
    fn saved_file_is_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(dir.path().join("config.json"));
        store.save(&sample_config()).unwrap();
        let mode = std::fs::metadata(dir.path().join("config.json")).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn corrupt_file_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, "{not json").unwrap();
        let store = ConfigStore::new(path);
        assert!(matches!(store.load(), Err(CoreError::Json(_))));
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p ssh-tunnel-core config`
Expected: 编译失败

- [ ] **Step 3: 实现 error.rs**

```rust
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
    fn from(e: russh::Error) -> Self { CoreError::Ssh(e.to_string()) }
}

impl From<russh::keys::Error> for CoreError {
    fn from(e: russh::keys::Error) -> Self { CoreError::Key(e.to_string()) }
}

impl From<keyring::Error> for CoreError {
    fn from(e: keyring::Error) -> Self { CoreError::Keyring(e.to_string()) }
}
```

- [ ] **Step 4: 实现 config.rs**

```rust
use crate::model::{Forward, Server, Settings};
use crate::CoreError;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppConfig {
    pub version: u32,
    pub servers: Vec<Server>,
    pub forwards: Vec<Forward>,
    #[serde(default)]
    pub settings: Settings,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self { version: 1, servers: vec![], forwards: vec![], settings: Settings::default() }
    }
}

pub struct ConfigStore {
    path: PathBuf,
}

impl ConfigStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn load(&self) -> Result<AppConfig, CoreError> {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => Ok(serde_json::from_str(&text)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(AppConfig::default()),
            Err(e) => Err(e.into()),
        }
    }

    // 原子写：先写临时文件再 rename，避免中途断电留下半个 JSON
    pub fn save(&self, config: &AppConfig) -> Result<(), CoreError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        let text = serde_json::to_string_pretty(config)?;
        std::fs::write(&tmp, text)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::fs::Permissions::from_mode(0o600))?;
        }
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}
```

- [ ] **Step 5: 实现 paths.rs**

```rust
use std::path::PathBuf;

/// 配置目录：Windows = %APPDATA%\ssh-tunnel；其余 = $XDG_CONFIG_HOME 或 ~/.config/ssh-tunnel
pub fn config_dir() -> PathBuf {
    #[cfg(windows)]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            return PathBuf::from(appdata).join("ssh-tunnel");
        }
    }
    #[cfg(not(windows))]
    {
        if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
            return PathBuf::from(xdg).join("ssh-tunnel");
        }
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(".config").join("ssh-tunnel");
        }
    }
    // 兜底：当前目录，仅在上面环境变量全缺失时才会走到
    PathBuf::from(".ssh-tunnel")
}
```

- [ ] **Step 6: 跑测试确认通过**

Run: `cargo test -p ssh-tunnel-core`
Expected: 全部通过（含 Task 1 的 3 个）

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat: 配置存储、错误类型与配置目录解析"
```

### Task 3: 凭据存储（SecretStore）

**Files:**
- Create: `core/src/secrets.rs`
- Test: `core/src/secrets.rs` 内 `#[cfg(test)]`

**Interfaces:**
- Produces: `SecretKind::{Password, Key, KeyPassphrase}`；`secret_account(server_id, kind) -> String`（格式 `<serverId>:password|key|key_passphrase`，keyring 的 service 固定 `"ssh-tunnel"`）；`trait SecretStore: Send + Sync { fn get(&self, server_id: &str, kind: SecretKind) -> Result<Option<String>, CoreError>; fn set(&self, server_id: &str, kind: SecretKind, value: &str) -> Result<(), CoreError>; fn delete(&self, server_id: &str, kind: SecretKind) -> Result<(), CoreError>; }`；`MemorySecretStore::new()`（测试用）；`KeyringStore::new()`。注意：keyring 调用是阻塞的，异步代码里调用处必须 `tokio::task::spawn_blocking`。

- [ ] **Step 1: 写失败测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_naming() {
        assert_eq!(secret_account("s1", SecretKind::Password), "s1:password");
        assert_eq!(secret_account("s1", SecretKind::Key), "s1:key");
        assert_eq!(secret_account("s1", SecretKind::KeyPassphrase), "s1:key_passphrase");
    }

    #[test]
    fn memory_store_roundtrip() {
        let store = MemorySecretStore::new();
        assert_eq!(store.get("s1", SecretKind::Password).unwrap(), None);
        store.set("s1", SecretKind::Password, "pw").unwrap();
        assert_eq!(store.get("s1", SecretKind::Password).unwrap().as_deref(), Some("pw"));
        store.set("s1", SecretKind::Password, "pw2").unwrap();
        assert_eq!(store.get("s1", SecretKind::Password).unwrap().as_deref(), Some("pw2"));
        store.delete("s1", SecretKind::Password).unwrap();
        assert_eq!(store.get("s1", SecretKind::Password).unwrap(), None);
        // 不同 kind / 不同 server 互不影响
        store.set("s1", SecretKind::Key, "k").unwrap();
        store.set("s2", SecretKind::Key, "k2").unwrap();
        assert_eq!(store.get("s1", SecretKind::Key).unwrap().as_deref(), Some("k"));
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p ssh-tunnel-core secrets`
Expected: 编译失败

- [ ] **Step 3: 实现 secrets.rs**

```rust
use crate::CoreError;
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SecretKind {
    Password,
    Key,
    KeyPassphrase,
}

/// keyring 条目的 account 名；service 固定为 "ssh-tunnel"
pub fn secret_account(server_id: &str, kind: SecretKind) -> String {
    let suffix = match kind {
        SecretKind::Password => "password",
        SecretKind::Key => "key",
        SecretKind::KeyPassphrase => "key_passphrase",
    };
    format!("{server_id}:{suffix}")
}

/// 同步 trait：keyring 本身是阻塞 IO，调用方负责 spawn_blocking
pub trait SecretStore: Send + Sync {
    fn get(&self, server_id: &str, kind: SecretKind) -> Result<Option<String>, CoreError>;
    fn set(&self, server_id: &str, kind: SecretKind, value: &str) -> Result<(), CoreError>;
    fn delete(&self, server_id: &str, kind: SecretKind) -> Result<(), CoreError>;
}

const SERVICE: &str = "ssh-tunnel";

pub struct KeyringStore;

impl KeyringStore {
    pub fn new() -> Self { Self }
    fn entry(server_id: &str, kind: SecretKind) -> Result<keyring::Entry, CoreError> {
        Ok(keyring::Entry::new(SERVICE, &secret_account(server_id, kind))?)
    }
}

impl SecretStore for KeyringStore {
    fn get(&self, server_id: &str, kind: SecretKind) -> Result<Option<String>, CoreError> {
        match Self::entry(server_id, kind)?.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    fn set(&self, server_id: &str, kind: SecretKind, value: &str) -> Result<(), CoreError> {
        Self::entry(server_id, kind)?.set_password(value)?;
        Ok(())
    }
    fn delete(&self, server_id: &str, kind: SecretKind) -> Result<(), CoreError> {
        match Self::entry(server_id, kind)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

/// 测试与无桌面环境下的内存实现
#[derive(Default)]
pub struct MemorySecretStore {
    inner: Mutex<HashMap<(String, SecretKind), String>>,
}

impl MemorySecretStore {
    pub fn new() -> Self { Self::default() }
}

impl SecretStore for MemorySecretStore {
    fn get(&self, server_id: &str, kind: SecretKind) -> Result<Option<String>, CoreError> {
        Ok(self.inner.lock().unwrap().get(&(server_id.to_string(), kind)).cloned())
    }
    fn set(&self, server_id: &str, kind: SecretKind, value: &str) -> Result<(), CoreError> {
        self.inner.lock().unwrap().insert((server_id.to_string(), kind), value.to_string());
        Ok(())
    }
    fn delete(&self, server_id: &str, kind: SecretKind) -> Result<(), CoreError> {
        self.inner.lock().unwrap().remove(&(server_id.to_string(), kind));
        Ok(())
    }
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p ssh-tunnel-core secrets`
Expected: 2 passed

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: 凭据存储抽象（keyring + 内存实现）"
```

---

### Task 4: known_hosts 封装

**Files:**
- Modify: `core/src/known_hosts.rs`
- Test: `core/src/known_hosts.rs` 内 `#[cfg(test)]`

**Interfaces:**
- Produces: `HostKeyStatus::{Trusted, Unknown, Changed}`；`KnownHosts::new(path: PathBuf)`、`check(&self, host: &str, port: u16, key: &russh::keys::PublicKey) -> Result<HostKeyStatus, CoreError>`（文件不存在 → Unknown）、`record(&self, host, port, key) -> Result<(), CoreError>`（追加标准 known_hosts 行）。直接复用 russh 自带的 `check_known_hosts_path` / `learn_known_hosts_path`，文件用标准 OpenSSH 格式。

- [ ] **Step 1: 写失败测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    // 一次性生成的测试密钥（仅测试用，对应公钥由此私钥现算）
    const TEST_KEY_PEM: &str = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW\nQyNTUxOQAAACA+DoeanTBWDxrSxMpB7n99cAYBH+KJWA6k1w3F5kjhVAAAAJg/lSznP5Us\n5wAAAAtzc2gtZWQyNTUxOQAAACA+DoeanTBWDxrSxMpB7n99cAYBH+KJWA6k1w3F5kjhVA\nAAAEBwkxfG4Qcvs76Hgt3QhMo3pA3dpAVPHmmq3IvfR5hhxD4Oh5qdMFYPGtLEykHuf31w\nBgEf4olYDqTXDcXmSOFUAAAAD3NzaC10dW5uZWwtdGVzdAECAwQFBg==\n-----END OPENSSH PRIVATE KEY-----\n";

    fn test_pubkey() -> russh::keys::PublicKey {
        russh::keys::decode_secret_key(TEST_KEY_PEM, None).unwrap().public_key().clone()
    }

    // 另一把不同的密钥，用于模拟 host key 变更
    fn other_pubkey() -> russh::keys::PublicKey {
        use russh::keys::ssh_key::rand_core::OsRng;
        russh::keys::PrivateKey::random(&mut OsRng, russh::keys::Algorithm::Ed25519)
            .unwrap().public_key().clone()
    }

    #[test]
    fn unknown_then_record_then_trusted() {
        let dir = tempfile::tempdir().unwrap();
        let kh = KnownHosts::new(dir.path().join("known_hosts"));
        let key = test_pubkey();
        assert_eq!(kh.check("h1", 22, &key).unwrap(), HostKeyStatus::Unknown);
        kh.record("h1", 22, &key).unwrap();
        assert_eq!(kh.check("h1", 22, &key).unwrap(), HostKeyStatus::Trusted);
    }

    #[test]
    fn changed_key_detected() {
        let dir = tempfile::tempdir().unwrap();
        let kh = KnownHosts::new(dir.path().join("known_hosts"));
        kh.record("h1", 22, &test_pubkey()).unwrap();
        assert_eq!(kh.check("h1", 22, &other_pubkey()).unwrap(), HostKeyStatus::Changed);
    }

    #[test]
    fn different_host_port_independent() {
        let dir = tempfile::tempdir().unwrap();
        let kh = KnownHosts::new(dir.path().join("known_hosts"));
        kh.record("h1", 22, &test_pubkey()).unwrap();
        assert_eq!(kh.check("h1", 2222, &test_pubkey()).unwrap(), HostKeyStatus::Unknown);
        assert_eq!(kh.check("h2", 22, &test_pubkey()).unwrap(), HostKeyStatus::Unknown);
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p ssh-tunnel-core known_hosts`
Expected: 编译失败

- [ ] **Step 3: 实现 known_hosts.rs**

```rust
use crate::CoreError;
use russh::keys::PublicKey;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostKeyStatus {
    Trusted,
    Unknown,
    Changed,
}

/// 标准 OpenSSH known_hosts 格式的薄封装，文件放在配置目录下
pub struct KnownHosts {
    path: PathBuf,
}

impl KnownHosts {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn check(&self, host: &str, port: u16, key: &PublicKey) -> Result<HostKeyStatus, CoreError> {
        if !self.path.exists() {
            return Ok(HostKeyStatus::Unknown);
        }
        match russh::keys::check_known_hosts_path(host, port, key, &self.path) {
            Ok(true) => Ok(HostKeyStatus::Trusted),
            Ok(false) => Ok(HostKeyStatus::Unknown),
            Err(russh::keys::Error::KeyChanged { .. }) => Ok(HostKeyStatus::Changed),
            Err(e) => Err(e.into()),
        }
    }

    pub fn record(&self, host: &str, port: u16, key: &PublicKey) -> Result<(), CoreError> {
        russh::keys::learn_known_hosts_path(host, port, key, &self.path)?;
        Ok(())
    }
}
```

注意：`PrivateKey::random` 需要 ssh_key 的 rand 支持。若 `russh::keys::ssh_key::rand_core::OsRng` 不可直接用（feature 未开），改为在 `core/Cargo.toml` 的 `[dev-dependencies]` 加 `ssh_key = { version = "0.6", features = ["rand_core"] }` 并用 `ssh_key::rand_core::OsRng`；russh re-export 的 `ssh_key` 版本需与之一致（`cargo tree | grep ssh_key` 确认）。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p ssh-tunnel-core known_hosts`
Expected: 3 passed

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: host key 记录与变更检测"
```

### Task 5: 测试基础设施 —— 进程内 echo SSH 服务器

**Files:**
- Create: `core/tests/support/mod.rs`
- Test: `core/tests/echo_server_smoke.rs`

**Interfaces:**
- Produces（供 Task 6-11 的集成测试使用）:
  - `TEST_PASSWORD: &str`（`"test-password-123"`）
  - `TEST_SERVER_HOST_KEY: &str`（内嵌 PEM，测试服务器用）
  - `TEST_CLIENT_KEY: &str`（内嵌 PEM，客户端密钥认证测试用，与 Task 4 同一把）
  - `TestServerOpts { password: Option<&'static str>, accept_keys: Vec<String> }`
  - `start_ssh_server(opts: TestServerOpts) -> TestServerHandle`，`TestServerHandle { addr: SocketAddr, shutdown: RunningServerHandle }`
  - `start_tcp_echo() -> SocketAddr`（纯 TCP echo，充当转发目标）
- 测试服务器行为：密码/公钥认证按 opts 校验；`channel_open_direct_tcpip` 接受后桥接到真实目标地址（因此能端到端转发到 `start_tcp_echo`）；`tcpip_forward` 后在服务器侧绑定监听，收到连接即向客户端开 `forwarded-tcpip` 通道并桥接（模拟真实 -R）。

- [ ] **Step 1: 写 support/mod.rs**

```rust
//! 进程内测试 SSH 服务器：密码/公钥认证 + direct-tcpip 桥接 + tcpip-forward（-R）模拟
use russh::keys::{decode_secret_key, HashAlg, PrivateKey, PublicKey};
use russh::server::{self, Auth, Config, Msg, RunningServerHandle, Server as _, Session};
use russh::{Channel, ChannelOpenFailure, MethodSet};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;

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
    let mut server = TestServer { opts };
    let mut running = server.run_on_socket(config, &listener);
    let shutdown = running.handle();
    tokio::spawn(async move {
        let _ = running.await;
    });
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
}

impl server::Server for TestServer {
    type Handler = TestHandler;
    fn new_client(&mut self, _peer: Option<SocketAddr>) -> TestHandler {
        TestHandler { opts: self.opts.clone() }
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
        tokio::spawn(async move {
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
        Ok(true)
    }
}
```

- [ ] **Step 2: 写冒烟测试（先失败）**

`core/tests/echo_server_smoke.rs`：
```rust
mod support;

use russh::client::{self, Handler};
use russh::keys::{HashAlg, PublicKeyOrCertificate};
use std::sync::Arc;
use support::*;

struct SmokeHandler;

impl Handler for SmokeHandler {
    type Error = russh::Error;
    async fn check_server_key(&mut self, _key: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

#[tokio::test]
async fn connect_and_password_auth() {
    let server = start_ssh_server(TestServerOpts {
        password: Some(TEST_PASSWORD),
        accept_keys: vec![],
    })
    .await;
    let config = Arc::new(client::Config::default());
    let mut handle = client::connect(config, server.addr, SmokeHandler).await.unwrap();
    let result = handle.authenticate_password("u", TEST_PASSWORD).await.unwrap();
    assert!(matches!(result, russh::auth::AuthResult::Success));
    server.shutdown.shutdown("test done".into());
}
```

- [ ] **Step 3: 跑测试确认失败（support 模块不存在）**

Run: `cargo test -p ssh-tunnel-core --test echo_server_smoke`
Expected: Step 1 写入后编译；若 Step 1 未写则编译失败（顺序上先写测试文件但 support 已在此任务同步实现；TDD 的"失败"体现在此测试驱动 support 实现修正）

- [ ] **Step 4: 跑通并修正**

Run: `cargo test -p ssh-tunnel-core --test echo_server_smoke`
Expected: PASS。若 russh server API 细节不符（如 `Auth` 变体字段、导入路径），按编译器提示修正 support/mod.rs——上方代码已按 0.63.1 源码核对。

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "test: 进程内 echo SSH 服务器测试设施"
```

---

### Task 6: SSH client 封装（连接 + 认证 + host key + 断线通知）

**Files:**
- Modify: `core/src/ssh/mod.rs`
- Create: `core/src/ssh/client.rs`
- Test: `core/tests/client_connect.rs`

**Interfaces:**
- Consumes: Task 1-5 全部
- Produces:
  - `pub type HostKeyDecider = Arc<dyn Fn(HostKeyInfo) -> Pin<Box<dyn Future<Output = bool> + Send>> + Send + Sync>`；`HostKeyInfo { host, port, fingerprint: String, is_mismatch: bool }`
  - `pub struct RemoteTarget { forward_id: String, target_host: String, target_port: u16 }`（Clone）
  - `pub struct ClientHandler`（内部用）
  - `pub struct Connection { handle: client::Handle<ClientHandler>, disconnect_rx: mpsc::Receiver<String>, remote_forwards: Arc<RwLock<HashMap<u32, RemoteTarget>>> }`
  - `pub async fn connect(server: &Server, secrets: Arc<dyn SecretStore>, known_hosts: Arc<Mutex<KnownHosts>>, decider: HostKeyDecider) -> Result<Connection, CoreError>`
  - `pub struct ChannelOpener { tx: mpsc::Sender<OpenChannelRequest> }`（Clone）；`OpenChannelRequest { target_host: String, target_port: u32, respond: oneshot::Sender<Result<Channel<Msg>, CoreError>> }`；`ChannelOpener::open(&self, host: &str, port: u32) -> Result<Channel<Msg>, CoreError>`。actor 持有 `mpsc::Receiver<OpenChannelRequest>`，收到后调 `handle.channel_open_direct_tcpip` 并回传结果（Handle 非 Clone 的解法）。

- [ ] **Step 1: 写失败测试**

`core/tests/client_connect.rs`：
```rust
mod support;

use ssh_tunnel_core::known_hosts::KnownHosts;
use ssh_tunnel_core::secrets::{MemorySecretStore, SecretKind, SecretStore};
use ssh_tunnel_core::ssh::client::{connect, HostKeyDecider};
use ssh_tunnel_core::model::{AuthMethod, Server};
use ssh_tunnel_core::CoreError;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::Mutex;
use support::*;

fn test_server(addr: std::net::SocketAddr, auth: AuthMethod) -> Server {
    Server {
        id: "s1".into(), name: "t".into(), host: addr.ip().to_string(),
        port: addr.port(), username: "u".into(), auth,
    }
}

fn always_trust() -> HostKeyDecider {
    Arc::new(|_info| Box::pin(async { true }))
}

fn always_reject() -> HostKeyDecider {
    Arc::new(|_info| Box::pin(async { false }))
}

fn temp_known_hosts() -> Arc<Mutex<KnownHosts>> {
    let dir = tempfile::tempdir().unwrap();
    // 用 leak 保活临时目录：测试生命周期短，进程退出即清理
    let path = Box::leak(Box::new(dir)).path().join("known_hosts");
    Arc::new(Mutex::new(KnownHosts::new(path)))
}

#[tokio::test]
async fn password_auth_success() {
    let server = start_ssh_server(TestServerOpts { password: Some(TEST_PASSWORD), accept_keys: vec![] }).await;
    let secrets = Arc::new(MemorySecretStore::new());
    secrets.set("s1", SecretKind::Password, TEST_PASSWORD).unwrap();
    let conn = connect(&test_server(server.addr, AuthMethod::Password), secrets, temp_known_hosts(), always_trust()).await;
    assert!(conn.is_ok(), "连接应成功: {:?}", conn.err());
}

#[tokio::test]
async fn password_auth_failure() {
    let server = start_ssh_server(TestServerOpts { password: Some(TEST_PASSWORD), accept_keys: vec![] }).await;
    let secrets = Arc::new(MemorySecretStore::new());
    secrets.set("s1", SecretKind::Password, "wrong").unwrap();
    let result = connect(&test_server(server.addr, AuthMethod::Password), secrets, temp_known_hosts(), always_trust()).await;
    assert!(matches!(result, Err(CoreError::Auth(_))));
}

#[tokio::test]
async fn missing_password_is_auth_error() {
    let server = start_ssh_server(TestServerOpts { password: Some(TEST_PASSWORD), accept_keys: vec![] }).await;
    let secrets = Arc::new(MemorySecretStore::new());
    let result = connect(&test_server(server.addr, AuthMethod::Password), secrets, temp_known_hosts(), always_trust()).await;
    assert!(matches!(result, Err(CoreError::Auth(_))));
}

#[tokio::test]
async fn key_data_auth_success() {
    // 授权公钥由测试客户端私钥现算
    let pubkey = russh::keys::decode_secret_key(TEST_CLIENT_KEY, None)
        .unwrap().public_key().to_openssh().unwrap();
    let server = start_ssh_server(TestServerOpts { password: None, accept_keys: vec![pubkey] }).await;
    let secrets = Arc::new(MemorySecretStore::new());
    secrets.set("s1", SecretKind::Key, TEST_CLIENT_KEY).unwrap();
    let conn = connect(&test_server(server.addr, AuthMethod::KeyData), secrets, temp_known_hosts(), always_trust()).await;
    assert!(conn.is_ok(), "密钥认证应成功: {:?}", conn.err());
}

#[tokio::test]
async fn untrusted_host_key_aborts() {
    let server = start_ssh_server(TestServerOpts { password: Some(TEST_PASSWORD), accept_keys: vec![] }).await;
    let secrets = Arc::new(MemorySecretStore::new());
    secrets.set("s1", SecretKind::Password, TEST_PASSWORD).unwrap();
    let result = connect(&test_server(server.addr, AuthMethod::Password), secrets, temp_known_hosts(), always_reject()).await;
    assert!(matches!(result, Err(CoreError::HostKeyRejected)));
}

#[tokio::test]
async fn trusted_key_persisted_and_reused() {
    let server = start_ssh_server(TestServerOpts { password: Some(TEST_PASSWORD), accept_keys: vec![] }).await;
    let secrets = Arc::new(MemorySecretStore::new());
    secrets.set("s1", SecretKind::Password, TEST_PASSWORD).unwrap();
    let kh = temp_known_hosts();
    let s = test_server(server.addr, AuthMethod::Password);
    // 第一次:decider 信任并记录;第二次:decider 拒绝也应成功(已信任)
    connect(&s, secrets.clone(), kh.clone(), always_trust()).await.unwrap();
    let conn2 = connect(&s, secrets, kh, always_reject()).await;
    assert!(conn2.is_ok(), "已记录的 host key 不应再询问: {:?}", conn2.err());
}

#[tokio::test]
async fn disconnect_notified_when_server_gone() {
    let server = start_ssh_server(TestServerOpts { password: Some(TEST_PASSWORD), accept_keys: vec![] }).await;
    let secrets = Arc::new(MemorySecretStore::new());
    secrets.set("s1", SecretKind::Password, TEST_PASSWORD).unwrap();
    let mut conn = connect(&test_server(server.addr, AuthMethod::Password), secrets, temp_known_hosts(), always_trust()).await.unwrap();
    server.shutdown.shutdown("bye".into());
    let msg = tokio::time::timeout(std::time::Duration::from_secs(5), conn.disconnect_rx.recv()).await;
    assert!(msg.is_ok(), "服务器关停后应收到断线通知");
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p ssh-tunnel-core --test client_connect`
Expected: 编译失败（ssh::client 模块为空）

- [ ] **Step 3: 实现 ssh/client.rs**

```rust
//! russh client 封装：认证、host key 校验、断线通知、-R 通道分发
use crate::known_hosts::{HostKeyStatus, KnownHosts};
use crate::model::{AuthMethod, Server};
use crate::secrets::{SecretKind, SecretStore};
use crate::CoreError;
use russh::client::{self, ChannelOpenHandle, Session};
use russh::keys::{HashAlg, PrivateKey, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh::{Channel, ChannelOpenFailure, ChannelMsg, Disconnect};
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
    pub respond: oneshot::Sender<Result<Channel<ChannelMsg>, CoreError>>,
}

#[derive(Clone)]
pub struct ChannelOpener {
    tx: mpsc::Sender<OpenChannelRequest>,
}

impl ChannelOpener {
    pub fn new(tx: mpsc::Sender<OpenChannelRequest>) -> Self {
        Self { tx }
    }

    pub async fn open(&self, host: &str, port: u32) -> Result<Channel<ChannelMsg>, CoreError> {
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
                if trusted {
                    self.known_hosts.lock().await.record(&self.host, self.port, &pubkey)?;
                }
                Ok(trusted)
            }
        }
    }

    // 服务器侧来连接（-R）：按 connected_port 找到本地目标并桥接
    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: Channel<ChannelMsg>,
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
            client::DisconnectReason::ReceivedDisconnect(info) => info.description,
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
    // 网络抖动场景给连接与认证一个上限，避免 actor 卡死
    let connect_future = client::connect(config, (server.host.as_str(), server.port), handler);
    let mut handle = tokio::time::timeout(Duration::from_secs(10), connect_future)
        .await
        .map_err(|_| CoreError::Ssh("连接超时".into()))??;
    authenticate(&mut handle, server, secrets).await?;
    Ok(Connection { handle, disconnect_rx, remote_forwards })
}

async fn authenticate(
    handle: &mut client::Handle<ClientHandler>,
    server: &Server,
    secrets: Arc<dyn SecretStore>,
) -> Result<(), CoreError> {
    match &server.auth {
        AuthMethod::Password => {
            let password = secrets
                .get(&server.id, SecretKind::Password)?
                .ok_or_else(|| CoreError::Auth("未保存密码".into()))?;
            let result = handle.authenticate_password(&server.username, password).await?;
            ensure_success(result)
        }
        AuthMethod::KeyFile { path } => {
            let pass = secrets.get(&server.id, SecretKind::KeyPassphrase)?;
            let key = russh::keys::load_secret_key(path, pass.as_deref())
                .map_err(|e| CoreError::Key(format!("读取密钥文件 {path} 失败: {e}")))?;
            auth_with_key(handle, &server.username, key).await
        }
        AuthMethod::KeyData => {
            let data = secrets
                .get(&server.id, SecretKind::Key)?
                .ok_or_else(|| CoreError::Auth("未保存密钥内容".into()))?;
            let pass = secrets.get(&server.id, SecretKind::KeyPassphrase)?;
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

fn ensure_success(result: russh::auth::AuthResult) -> Result<(), CoreError> {
    match result {
        russh::auth::AuthResult::Success => Ok(()),
        russh::auth::AuthResult::Failure { .. } => Err(CoreError::Auth("用户名或凭据不正确".into())),
    }
}
```

`ssh/mod.rs`：
```rust
pub mod actor;
pub mod client;
pub mod manager;
```
（actor.rs / manager.rs 空文件占位，Task 10/11 填实。）

注意：`SecretStore` 是同步 trait，而 KeyringStore 的底层调用（dbus/Credential Manager）是阻塞 IO。`authenticate` 内所有 `secrets.get(...)` 必须用 `spawn_blocking` 包裹，避免阻塞 tokio worker：

```rust
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
```

`authenticate` 中三处 `secrets.get` 全部改为 `secret_get(&secrets, &server.id, SecretKind::Xxx).await?`（保留原错误映射逻辑，如 `ok_or_else(|| CoreError::Auth(...))` 跟在 `?` 之后）。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p ssh-tunnel-core --test client_connect`
Expected: 7 passed。若 `Channel<Msg>`/`ChannelMsg` 类型路径不符（russh 0.63 中 client 的 Channel 类型参数为 `russh::ChannelMsg`，re-export 于 crate 根），按编译器提示调整 use。

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: SSH client 封装（认证、host key 校验、断线通知、-R 分发）"
```

### Task 7: 本地转发（-L）

**Files:**
- Modify: `core/src/forward/mod.rs`
- Create: `core/src/forward/local.rs`
- Test: `core/tests/local_forward.rs`

**Interfaces:**
- Consumes: `ssh::client::{connect, Connection, ChannelOpener, OpenChannelRequest, HostKeyDecider}`、`start_ssh_server`、`start_tcp_echo`
- Produces:
  - `pub async fn bind_listener(addr: &str, port: u16) -> Result<TcpListener, CoreError>`（AddrInUse → `CoreError::PortInUse(port)`）
  - `pub fn spawn_local_forward(listener: TcpListener, opener: ChannelOpener, target_host: String, target_port: u16) -> JoinHandle<()>`

- [ ] **Step 1: 写失败测试**

`core/tests/local_forward.rs`：
```rust
mod support;

use ssh_tunnel_core::forward::local::{bind_listener, spawn_local_forward};
use ssh_tunnel_core::known_hosts::KnownHosts;
use ssh_tunnel_core::model::{AuthMethod, Server};
use ssh_tunnel_core::secrets::{MemorySecretStore, SecretKind, SecretStore};
use ssh_tunnel_core::ssh::client::{connect, ChannelOpener, OpenChannelRequest};
use ssh_tunnel_core::CoreError;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, Mutex};
use support::*;

async fn connected_opener(ssh_addr: std::net::SocketAddr) -> (ChannelOpener, tokio::task::JoinHandle<()>) {
    let secrets = Arc::new(MemorySecretStore::new());
    secrets.set("s1", SecretKind::Password, TEST_PASSWORD).unwrap();
    let server = Server {
        id: "s1".into(), name: "t".into(), host: ssh_addr.ip().to_string(),
        port: ssh_addr.port(), username: "u".into(), auth: AuthMethod::Password,
    };
    let dir = tempfile::tempdir().unwrap();
    let kh = Arc::new(Mutex::new(KnownHosts::new(Box::leak(Box::new(dir)).path().join("kh"))));
    let decider: ssh_tunnel_core::ssh::client::HostKeyDecider = Arc::new(|_| Box::pin(async { true }));
    let conn = connect(&server, secrets, kh, decider).await.unwrap();
    // 模拟 actor:持有 handle,响应开通道请求
    let (tx, mut rx) = mpsc::channel::<OpenChannelRequest>(32);
    let mut handle = conn.handle;
    let pump = tokio::spawn(async move {
        while let Some(req) = rx.recv().await {
            let r = handle
                .channel_open_direct_tcpip(req.target_host, req.target_port, "127.0.0.1", 0)
                .await
                .map_err(CoreError::from);
            let _ = req.respond.send(r);
        }
    });
    (ChannelOpener::new(tx), pump)
}

#[tokio::test]
async fn local_forward_pipes_data() {
    let echo = start_tcp_echo().await;
    let ssh = start_ssh_server(TestServerOpts { password: Some(TEST_PASSWORD), accept_keys: vec![] }).await;
    let (opener, _pump) = connected_opener(ssh.addr).await;

    let listener = bind_listener("127.0.0.1", 0).await.unwrap();
    let local_addr = listener.local_addr().unwrap();
    let _task = spawn_local_forward(listener, opener, echo.ip().to_string(), echo.port());

    let mut client = tokio::net::TcpStream::connect(local_addr).await.unwrap();
    client.write_all(b"hello tunnel").await.unwrap();
    let mut buf = vec![0u8; 12];
    client.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"hello tunnel");
}

#[tokio::test]
async fn bind_conflict_reports_port() {
    let first = bind_listener("127.0.0.1", 0).await.unwrap();
    let port = first.local_addr().unwrap().port();
    let result = bind_listener("127.0.0.1", port).await;
    assert!(matches!(result, Err(CoreError::PortInUse(p)) if p == port));
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p ssh-tunnel-core --test local_forward`
Expected: 编译失败

- [ ] **Step 3: 实现 forward/local.rs 与 forward/mod.rs**

`forward/mod.rs`：
```rust
pub mod local;
pub mod remote;
pub mod socks;
```
（remote.rs / socks.rs 空文件占位，Task 8/9 填实。）

`forward/local.rs`：
```rust
use crate::ssh::client::ChannelOpener;
use crate::CoreError;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

pub async fn bind_listener(addr: &str, port: u16) -> Result<TcpListener, CoreError> {
    TcpListener::bind((addr, port)).await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::AddrInUse {
            CoreError::PortInUse(port)
        } else {
            CoreError::Io(e)
        }
    })
}

/// 本地转发 accept 循环。listener 跨重连存活:每次 accept 时才向 actor 请求开通道,
/// 因此连接断开重建后无需重新绑定端口(避免重连时端口竞争)
pub fn spawn_local_forward(
    listener: TcpListener,
    opener: ChannelOpener,
    target_host: String,
    target_port: u16,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else { break };
            let opener = opener.clone();
            let (host, port) = (target_host.clone(), target_port as u32);
            tokio::spawn(async move {
                match opener.open(&host, port).await {
                    Ok(channel) => {
                        let mut stream = channel.into_stream();
                        let _ = tokio::io::copy_bidirectional(&mut stream, &mut socket).await;
                    }
                    Err(e) => tracing::warn!("开 direct-tcpip 通道失败: {e}"),
                }
            });
        }
    })
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p ssh-tunnel-core --test local_forward`
Expected: 2 passed

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: 本地转发(-L)与端口冲突错误"
```

---

### Task 8: 动态转发（-D / SOCKS5）

**Files:**
- Modify: `core/src/socks5.rs`
- Create: `core/src/forward/socks.rs`
- Test: `core/tests/socks_forward.rs`

**Interfaces:**
- Consumes: 同 Task 7
- Produces:
  - `pub async fn socks5_accept_target(stream: &mut TcpStream) -> Result<(String, u16), CoreError>`（读握手+CONNECT 请求，返回目标 host/port；不支持的命令/版本直接断开并返回 Err）
  - `pub async fn socks5_reply(stream: &mut TcpStream, success: bool) -> Result<(), CoreError>`
  - `pub fn spawn_socks_forward(listener: TcpListener, opener: ChannelOpener) -> JoinHandle<()>`

- [ ] **Step 1: 写失败测试**

`core/tests/socks_forward.rs`：
```rust
mod support;

use ssh_tunnel_core::forward::local::bind_listener;
use ssh_tunnel_core::forward::socks::spawn_socks_forward;
use ssh_tunnel_core::known_hosts::KnownHosts;
use ssh_tunnel_core::model::{AuthMethod, Server};
use ssh_tunnel_core::secrets::{MemorySecretStore, SecretKind, SecretStore};
use ssh_tunnel_core::ssh::client::{connect, ChannelOpener, OpenChannelRequest};
use ssh_tunnel_core::CoreError;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, Mutex};
use support::*;

// 手写最小 SOCKS5 客户端握手,验证服务端实现
async fn socks5_connect(client: &mut tokio::net::TcpStream, host: &str, port: u16) {
    client.write_all(&[0x05, 0x01, 0x00]).await.unwrap(); // VER=5, 1 method, no-auth
    let mut buf = [0u8; 2];
    client.read_exact(&mut buf).await.unwrap();
    assert_eq!(buf, [0x05, 0x00]);

    let mut req = vec![0x05, 0x01, 0x00, 0x03, host.len() as u8];
    req.extend_from_slice(host.as_bytes());
    req.extend_from_slice(&port.to_be_bytes());
    client.write_all(&req).await.unwrap();

    let mut reply = [0u8; 4];
    client.read_exact(&mut reply).await.unwrap();
    assert_eq!(reply[1], 0x00, "SOCKS5 应答应成功");
    // 消费 BND.ADDR(IPv4 4 字节) + BND.PORT(2 字节)
    let mut rest = [0u8; 6];
    client.read_exact(&mut rest).await.unwrap();
}

#[tokio::test]
async fn socks_forward_pipes_data() {
    let echo = start_tcp_echo().await;
    let ssh = start_ssh_server(TestServerOpts { password: Some(TEST_PASSWORD), accept_keys: vec![] }).await;

    let secrets = Arc::new(MemorySecretStore::new());
    secrets.set("s1", SecretKind::Password, TEST_PASSWORD).unwrap();
    let server = Server {
        id: "s1".into(), name: "t".into(), host: ssh.addr.ip().to_string(),
        port: ssh.addr.port(), username: "u".into(), auth: AuthMethod::Password,
    };
    let dir = tempfile::tempdir().unwrap();
    let kh = Arc::new(Mutex::new(KnownHosts::new(Box::leak(Box::new(dir)).path().join("kh"))));
    let decider: ssh_tunnel_core::ssh::client::HostKeyDecider = Arc::new(|_| Box::pin(async { true }));
    let conn = connect(&server, secrets, kh, decider).await.unwrap();
    let (tx, mut rx) = mpsc::channel::<OpenChannelRequest>(32);
    let mut handle = conn.handle;
    tokio::spawn(async move {
        while let Some(req) = rx.recv().await {
            let r = handle.channel_open_direct_tcpip(req.target_host, req.target_port, "127.0.0.1", 0).await.map_err(CoreError::from);
            let _ = req.respond.send(r);
        }
    });

    let listener = bind_listener("127.0.0.1", 0).await.unwrap();
    let socks_addr = listener.local_addr().unwrap();
    let _task = spawn_socks_forward(listener, ChannelOpener::new(tx));

    let mut client = tokio::net::TcpStream::connect(socks_addr).await.unwrap();
    socks5_connect(&mut client, &echo.ip().to_string(), echo.port()).await;
    client.write_all(b"via socks").await.unwrap();
    let mut buf = vec![0u8; 9];
    client.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"via socks");
}

#[tokio::test]
async fn rejects_unsupported_command() {
    // 不经过 SSH:直接测 SOCKS5 协议层(opener 用不到)
    let listener = bind_listener("127.0.0.1", 0).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, _rx) = mpsc::channel::<OpenChannelRequest>(1);
    let _task = spawn_socks_forward(listener, ChannelOpener::new(tx));

    let mut client = tokio::net::TcpStream::connect(addr).await.unwrap();
    client.write_all(&[0x05, 0x01, 0x00]).await.unwrap();
    let mut buf = [0u8; 2];
    client.read_exact(&mut buf).await.unwrap();
    // CMD=2 (BIND) 不支持
    client.write_all(&[0x05, 0x02, 0x00, 0x01, 0, 0, 0, 0, 0, 0]).await.unwrap();
    let mut reply = [0u8; 10];
    client.read_exact(&mut reply).await.unwrap();
    assert_eq!(reply[1], 0x07, "不支持的命令应回复 0x07");
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p ssh-tunnel-core --test socks_forward`
Expected: 编译失败

- [ ] **Step 3: 实现 socks5.rs**

```rust
//! 极简 SOCKS5 服务端：仅无认证 + CONNECT,够浏览器/git 等客户端用
use crate::CoreError;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

pub async fn socks5_accept_target(stream: &mut TcpStream) -> Result<(String, u16), CoreError> {
    let mut head = [0u8; 2];
    stream.read_exact(&mut head).await?;
    if head[0] != 0x05 {
        return Err(CoreError::Other("非 SOCKS5 握手".into()));
    }
    // 读方法列表并选择无认证(0x00)
    let mut methods = vec![0u8; head[1] as usize];
    stream.read_exact(&mut methods).await?;
    stream.write_all(&[0x05, 0x00]).await?;

    let mut req = [0u8; 4];
    stream.read_exact(&mut req).await?;
    if req[0] != 0x05 || req[1] != 0x01 {
        // REP=0x07: 命令不支持
        stream.write_all(&[0x05, 0x07, 0x00, 0x01, 0, 0, 0, 0, 0, 0]).await?;
        return Err(CoreError::Other("SOCKS5 仅支持 CONNECT".into()));
    }
    let host = match req[3] {
        0x01 => {
            let mut b = [0u8; 4];
            stream.read_exact(&mut b).await?;
            std::net::Ipv4Addr::from(b).to_string()
        }
        0x03 => {
            let mut len = [0u8; 1];
            stream.read_exact(&mut len).await?;
            let mut b = vec![0u8; len[0] as usize];
            stream.read_exact(&mut b).await?;
            String::from_utf8(b).map_err(|_| CoreError::Other("SOCKS5 域名非 UTF-8".into()))?
        }
        0x04 => {
            let mut b = [0u8; 16];
            stream.read_exact(&mut b).await?;
            std::net::Ipv6Addr::from(b).to_string()
        }
        _ => return Err(CoreError::Other("SOCKS5 未知地址类型".into())),
    };
    let mut port = [0u8; 2];
    stream.read_exact(&mut port).await?;
    Ok((host, u16::from_be_bytes(port)))
}

pub async fn socks5_reply(stream: &mut TcpStream, success: bool) -> Result<(), CoreError> {
    let rep = if success { 0x00 } else { 0x05 };
    stream.write_all(&[0x05, rep, 0x00, 0x01, 0, 0, 0, 0, 0, 0]).await?;
    Ok(())
}
```

- [ ] **Step 4: 实现 forward/socks.rs**

```rust
use crate::socks5::{socks5_accept_target, socks5_reply};
use crate::ssh::client::ChannelOpener;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

pub fn spawn_socks_forward(listener: TcpListener, opener: ChannelOpener) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else { break };
            let opener = opener.clone();
            tokio::spawn(async move {
                let result = async {
                    let (host, port) = socks5_accept_target(&mut socket).await?;
                    match opener.open(&host, port as u32).await {
                        Ok(channel) => {
                            socks5_reply(&mut socket, true).await?;
                            let mut stream = channel.into_stream();
                            let _ = tokio::io::copy_bidirectional(&mut stream, &mut socket).await;
                        }
                        Err(e) => {
                            tracing::warn!("SOCKS5 目标连接失败: {e}");
                            socks5_reply(&mut socket, false).await?;
                        }
                    }
                    Ok::<(), crate::CoreError>(())
                };
                if let Err(e) = result.await {
                    tracing::debug!("SOCKS5 会话结束: {e}");
                }
            });
        }
    })
}
```

- [ ] **Step 5: 跑测试确认通过**

Run: `cargo test -p ssh-tunnel-core --test socks_forward`
Expected: 2 passed

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat: 动态转发(-D)与极简 SOCKS5 服务端"
```

### Task 9: 远程转发（-R）

**Files:**
- Create: `core/src/forward/remote.rs`
- Test: `core/tests/remote_forward.rs`

**Interfaces:**
- Consumes: Task 6 的 `Connection.remote_forwards`、`RemoteTarget`、Task 5 测试服务器（已实现 `tcpip_forward`）
- Produces:
  - `pub async fn start_remote_forward(forward: &Forward, handle: &client::Handle<ClientHandler>, remote_forwards: &Arc<RwLock<HashMap<u32, RemoteTarget>>>) -> Result<(), CoreError>`（请求 `tcpip_forward(bind_addr, bind_port)`，成功后把 `RemoteTarget` 按分配端口写入 map）
  - `pub async fn stop_remote_forward(forward: &Forward, handle: &client::Handle<ClientHandler>, remote_forwards: &Arc<RwLock<HashMap<u32, RemoteTarget>>>) -> Result<(), CoreError>`（`cancel_tcpip_forward` + map 清理）

- [ ] **Step 1: 写失败测试**

`core/tests/remote_forward.rs`：
```rust
mod support;

use ssh_tunnel_core::forward::remote::{start_remote_forward, stop_remote_forward};
use ssh_tunnel_core::known_hosts::KnownHosts;
use ssh_tunnel_core::model::{AuthMethod, Forward, ForwardKind, Server};
use ssh_tunnel_core::secrets::{MemorySecretStore, SecretKind, SecretStore};
use ssh_tunnel_core::ssh::client::connect;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;
use support::*;

fn local_echo_target(echo: std::net::SocketAddr, ssh_addr: std::net::SocketAddr) -> Forward {
    Forward {
        id: "f1".into(), server_id: "s1".into(), name: "expose".into(),
        kind: ForwardKind::Remote,
        bind_addr: "127.0.0.1".into(), bind_port: 0, // 0 = 服务器分配,便于测试免冲突
        target_host: Some(echo.ip().to_string()), target_port: Some(echo.port()),
        auto_start: false,
    }
}

async fn setup() -> (support::TestServerHandle, ssh_tunnel_core::ssh::client::Connection, std::net::SocketAddr) {
    let echo = start_tcp_echo().await;
    let ssh = start_ssh_server(TestServerOpts { password: Some(TEST_PASSWORD), accept_keys: vec![] }).await;
    let secrets = Arc::new(MemorySecretStore::new());
    secrets.set("s1", SecretKind::Password, TEST_PASSWORD).unwrap();
    let server = Server {
        id: "s1".into(), name: "t".into(), host: ssh.addr.ip().to_string(),
        port: ssh.addr.port(), username: "u".into(), auth: AuthMethod::Password,
    };
    let dir = tempfile::tempdir().unwrap();
    let kh = Arc::new(Mutex::new(KnownHosts::new(Box::leak(Box::new(dir)).path().join("kh"))));
    let decider: ssh_tunnel_core::ssh::client::HostKeyDecider = Arc::new(|_| Box::pin(async { true }));
    let conn = connect(&server, secrets, kh, decider).await.unwrap();
    (ssh, conn, echo)
}

#[tokio::test]
async fn remote_forward_pipes_data() {
    let (_ssh, conn, echo) = setup().await;
    let fwd = local_echo_target(echo, _ssh.addr);
    start_remote_forward(&fwd, &conn.handle, &conn.remote_forwards).await.unwrap();

    // 从 map 里拿到服务器分配的端口
    let assigned = *conn.remote_forwards.read().await.keys().next().unwrap();
    assert_ne!(assigned, 0);

    // 连接"服务器侧"端口,数据应到达本地 echo
    let mut client = tokio::net::TcpStream::connect(("127.0.0.1", assigned as u16)).await.unwrap();
    client.write_all(b"remote!").await.unwrap();
    let mut buf = vec![0u8; 7];
    client.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"remote!");
}

#[tokio::test]
async fn stop_remote_forward_cleans_up() {
    let (_ssh, conn, echo) = setup().await;
    let fwd = local_echo_target(echo, _ssh.addr);
    start_remote_forward(&fwd, &conn.handle, &conn.remote_forwards).await.unwrap();
    let assigned = *conn.remote_forwards.read().await.keys().next().unwrap();

    // bind_port=0 时 cancel 用分配端口;实现需把分配端口写回 forward 副本
    let mut applied = fwd.clone();
    applied.bind_port = assigned as u16;
    stop_remote_forward(&applied, &conn.handle, &conn.remote_forwards).await.unwrap();
    assert!(conn.remote_forwards.read().await.is_empty());
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p ssh-tunnel-core --test remote_forward`
Expected: 编译失败

- [ ] **Step 3: 实现 forward/remote.rs**

```rust
//! 远程转发(-R):请求服务器监听,服务器侧来连接时经 forwarded-tcpip 通道回到客户端,
//! 由 ClientHandler::server_channel_open_forwarded_tcpip 按端口查 remote_forwards 桥接到本地目标
use crate::model::Forward;
use crate::ssh::client::{ClientHandler, RemoteTarget};
use crate::CoreError;
use russh::client;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

pub async fn start_remote_forward(
    forward: &Forward,
    handle: &client::Handle<ClientHandler>,
    remote_forwards: &Arc<RwLock<HashMap<u32, RemoteTarget>>>,
) -> Result<(), CoreError> {
    let target = RemoteTarget {
        forward_id: forward.id.clone(),
        target_host: forward.target_host.clone().unwrap_or_else(|| "127.0.0.1".into()),
        target_port: forward.target_port.ok_or_else(|| CoreError::Other("远程转发缺少目标端口".into()))?,
    };
    let assigned = handle.tcpip_forward(forward.bind_addr.clone(), forward.bind_port as u32).await?;
    remote_forwards.write().await.insert(assigned, target);
    Ok(())
}

pub async fn stop_remote_forward(
    forward: &Forward,
    handle: &client::Handle<ClientHandler>,
    remote_forwards: &Arc<RwLock<HashMap<u32, RemoteTarget>>>,
) -> Result<(), CoreError> {
    handle.cancel_tcpip_forward(forward.bind_addr.clone(), forward.bind_port as u32).await?;
    // 按 forward_id 清理,兼容 bind_port=0(分配端口)的情况
    remote_forwards.write().await.retain(|_, t| t.forward_id != forward.id);
    Ok(())
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p ssh-tunnel-core --test remote_forward`
Expected: 2 passed

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: 远程转发(-R)"
```

---

### Task 10: ServerActor —— 连接状态机与自动重连

**Files:**
- Modify: `core/src/ssh/actor.rs`
- Test: `core/tests/actor.rs`

**Interfaces:**
- Consumes: Task 1-9 全部
- Produces:
  - `pub enum ActorCommand { Connect, Disconnect, StartForward(Forward), StopForward { forward_id: String }, SetAutoReconnect(bool), Shutdown }`
  - `#[derive(Clone)] pub struct ActorHandle { tx: mpsc::Sender<ActorCommand> }`，方法 `send(cmd) -> Result<(), CoreError>`
  - `pub fn spawn_actor(server: Server, secrets: Arc<dyn SecretStore>, known_hosts: Arc<Mutex<KnownHosts>>, decider: HostKeyDecider, auto_reconnect: bool, events: broadcast::Sender<TunnelEvent>) -> ActorHandle`
  - `#[derive(Debug, Clone, Serialize)] #[serde(tag = "type", rename_all = "snake_case")] pub enum TunnelEvent { ServerStatus { server_id, status: ServerStatus, error: Option<String> }, ForwardStatus { forward_id, server_id, status: ForwardStatus, error: Option<String> } }`（定义在 `ssh/mod.rs`）
- 状态机：Disconnected ⇄ Connecting → Connected；连接丢失且 auto_reconnect → Reconnecting（指数退避 1s×2^n，30s 封顶）→ Connected/继续。手动 Disconnect 不触发重连。重连成功后自动恢复该服务器 running 的远程转发（local/socks 的 listener 跨重连存活，无需恢复）。

- [ ] **Step 1: 写失败测试**

`core/tests/actor.rs`：
```rust
mod support;

use ssh_tunnel_core::known_hosts::KnownHosts;
use ssh_tunnel_core::model::{AuthMethod, Forward, ForwardKind, Server, ServerStatus, ForwardStatus};
use ssh_tunnel_core::secrets::{MemorySecretStore, SecretKind, SecretStore};
use ssh_tunnel_core::ssh::actor::{spawn_actor, ActorCommand};
use ssh_tunnel_core::ssh::TunnelEvent;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, Mutex};
use support::*;

fn test_server(addr: std::net::SocketAddr) -> Server {
    Server {
        id: "s1".into(), name: "t".into(), host: addr.ip().to_string(),
        port: addr.port(), username: "u".into(), auth: AuthMethod::Password,
    }
}

fn decider() -> ssh_tunnel_core::ssh::client::HostKeyDecider {
    Arc::new(|_| Box::pin(async { true }) as _)
}

async fn wait_server_status(rx: &mut broadcast::Receiver<TunnelEvent>, want: ServerStatus) -> Option<String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        let ev = tokio::time::timeout(deadline - std::time::Instant::now(), rx.recv()).await.unwrap().unwrap();
        if let TunnelEvent::ServerStatus { status, error, .. } = ev {
            if status == want { return error; }
        }
    }
}

async fn wait_forward_status(rx: &mut broadcast::Receiver<TunnelEvent>, want: ForwardStatus) {
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        let ev = tokio::time::timeout(deadline - std::time::Instant::now(), rx.recv()).await.unwrap().unwrap();
        if let TunnelEvent::ForwardStatus { status, .. } = ev {
            if status == want { return; }
        }
    }
}

#[tokio::test]
async fn connect_and_disconnect() {
    let ssh = start_ssh_server(TestServerOpts { password: Some(TEST_PASSWORD), accept_keys: vec![] }).await;
    let secrets = Arc::new(MemorySecretStore::new());
    secrets.set("s1", SecretKind::Password, TEST_PASSWORD).unwrap();
    let (events, mut rx) = broadcast::channel(64);
    let kh = Arc::new(Mutex::new(KnownHosts::new(tempfile::tempdir().unwrap().path().join("kh"))));
    let actor = spawn_actor(test_server(ssh.addr), secrets, kh, decider(), false, events);

    actor.send(ActorCommand::Connect).unwrap();
    wait_server_status(&mut rx, ServerStatus::Connected).await;

    actor.send(ActorCommand::Disconnect).unwrap();
    wait_server_status(&mut rx, ServerStatus::Disconnected).await;
    actor.send(ActorCommand::Shutdown).unwrap();
}

#[tokio::test]
async fn start_forward_auto_connects_and_pipes() {
    let echo = start_tcp_echo().await;
    let ssh = start_ssh_server(TestServerOpts { password: Some(TEST_PASSWORD), accept_keys: vec![] }).await;
    let secrets = Arc::new(MemorySecretStore::new());
    secrets.set("s1", SecretKind::Password, TEST_PASSWORD).unwrap();
    let (events, mut rx) = broadcast::channel(64);
    let kh = Arc::new(Mutex::new(KnownHosts::new(tempfile::tempdir().unwrap().path().join("kh"))));
    let actor = spawn_actor(test_server(ssh.addr), secrets, kh, decider(), false, events);

    let listener_port = {
        // 先占一个临时端口拿到空闲端口号再释放,避免测试端口冲突
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let p = l.local_addr().unwrap().port();
        drop(l);
        p
    };
    let fwd = Forward {
        id: "f1".into(), server_id: "s1".into(), name: "mysql".into(),
        kind: ForwardKind::Local, bind_addr: "127.0.0.1".into(), bind_port: listener_port,
        target_host: Some(echo.ip().to_string()), target_port: Some(echo.port()),
        auto_start: false,
    };
    // 未连接时启动转发:应自动连服务器
    actor.send(ActorCommand::StartForward(fwd)).unwrap();
    wait_server_status(&mut rx, ServerStatus::Connected).await;
    wait_forward_status(&mut rx, ForwardStatus::Running).await;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut client = tokio::net::TcpStream::connect(("127.0.0.1", listener_port)).await.unwrap();
    client.write_all(b"ping").await.unwrap();
    let mut buf = vec![0u8; 4];
    client.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"ping");
    actor.send(ActorCommand::Shutdown).unwrap();
}

#[tokio::test]
async fn reconnects_after_server_restart_and_recovers_forward() {
    let echo = start_tcp_echo().await;
    let ssh = start_ssh_server(TestServerOpts { password: Some(TEST_PASSWORD), accept_keys: vec![] }).await;
    let addr = ssh.addr;
    let secrets = Arc::new(MemorySecretStore::new());
    secrets.set("s1", SecretKind::Password, TEST_PASSWORD).unwrap();
    let (events, mut rx) = broadcast::channel(64);
    let kh = Arc::new(Mutex::new(KnownHosts::new(tempfile::tempdir().unwrap().path().join("kh"))));
    let actor = spawn_actor(test_server(addr), secrets, kh, decider(), true, events);

    let listener_port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let p = l.local_addr().unwrap().port();
        drop(l);
        p
    };
    actor.send(ActorCommand::Connect).unwrap();
    wait_server_status(&mut rx, ServerStatus::Connected).await;
    let fwd = Forward {
        id: "f1".into(), server_id: "s1".into(), name: "mysql".into(),
        kind: ForwardKind::Local, bind_addr: "127.0.0.1".into(), bind_port: listener_port,
        target_host: Some(echo.ip().to_string()), target_port: Some(echo.port()),
        auto_start: false,
    };
    actor.send(ActorCommand::StartForward(fwd)).unwrap();
    wait_forward_status(&mut rx, ForwardStatus::Running).await;

    // 杀掉服务器,应进入重连
    ssh.shutdown.shutdown("boom".into());
    wait_server_status(&mut rx, ServerStatus::Reconnecting).await;

    // 同端口重启服务器(host key 相同 → known_hosts 仍然信任)
    let ssh2 = start_ssh_server_on(addr, TestServerOpts { password: Some(TEST_PASSWORD), accept_keys: vec![] }).await;
    wait_server_status(&mut rx, ServerStatus::Connected).await;

    // 转发自动恢复:数据仍通
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut client = tokio::net::TcpStream::connect(("127.0.0.1", listener_port)).await.unwrap();
    client.write_all(b"back").await.unwrap();
    let mut buf = vec![0u8; 4];
    client.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"back");
    actor.send(ActorCommand::Shutdown).unwrap();
    ssh2.shutdown.shutdown("done".into());
}

#[tokio::test]
async fn no_reconnect_when_disabled() {
    let ssh = start_ssh_server(TestServerOpts { password: Some(TEST_PASSWORD), accept_keys: vec![] }).await;
    let secrets = Arc::new(MemorySecretStore::new());
    secrets.set("s1", SecretKind::Password, TEST_PASSWORD).unwrap();
    let (events, mut rx) = broadcast::channel(64);
    let kh = Arc::new(Mutex::new(KnownHosts::new(tempfile::tempdir().unwrap().path().join("kh"))));
    let actor = spawn_actor(test_server(ssh.addr), secrets, kh, decider(), false, events);

    actor.send(ActorCommand::Connect).unwrap();
    wait_server_status(&mut rx, ServerStatus::Connected).await;
    ssh.shutdown.shutdown("boom".into());
    // auto_reconnect=false:应停在某终态,且 3 秒内不出现 Connected
    let mut saw_connected = false;
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while let Ok(Ok(ev)) = tokio::time::timeout(deadline - std::time::Instant::now(), rx.recv()).await.map_err(|_| ()).map(|r| r) {
        if let TunnelEvent::ServerStatus { status: ServerStatus::Connected, .. } = ev { saw_connected = true; }
    }
    assert!(!saw_connected);
    actor.send(ActorCommand::Shutdown).unwrap();
}
```

注意：测试用到 `start_ssh_server_on(addr, opts)`（指定端口重启）。在 `core/tests/support/mod.rs` 追加：

```rust
pub async fn start_ssh_server_on(addr: SocketAddr, opts: TestServerOpts) -> TestServerHandle {
    // 重试绑定:刚关停的端口可能处于 TIME_WAIT
    let listener = {
        let mut last_err = None;
        let mut listener = None;
        for _ in 0..20 {
            match TcpListener::bind(addr).await {
                Ok(l) => { listener = Some(l); break; }
                Err(e) => { last_err = Some(e); tokio::time::sleep(Duration::from_millis(100)).await; }
            }
        }
        listener.unwrap_or_else(|| panic!("绑定 {addr} 失败: {last_err:?}"))
    };
    let key = decode_secret_key(TEST_SERVER_HOST_KEY, None).unwrap();
    let config = Arc::new(Config { keys: vec![key], auth_rejection_time: Duration::ZERO, ..Default::default() });
    let mut server = TestServer { opts };
    let mut running = server.run_on_socket(config, &listener);
    let shutdown = running.handle();
    tokio::spawn(async move { let _ = running.await; });
    TestServerHandle { addr, shutdown }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p ssh-tunnel-core --test actor`
Expected: 编译失败

- [ ] **Step 3: 实现 ssh/mod.rs 的 TunnelEvent**

`core/src/ssh/mod.rs`：
```rust
pub mod actor;
pub mod client;
pub mod manager;

use crate::model::{ForwardStatus, ServerStatus};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TunnelEvent {
    ServerStatus {
        server_id: String,
        status: ServerStatus,
        error: Option<String>,
    },
    ForwardStatus {
        forward_id: String,
        server_id: String,
        status: ForwardStatus,
        error: Option<String>,
    },
}
```

- [ ] **Step 4: 实现 ssh/actor.rs**

```rust
//! 每服务器一个 actor:独占 russh Handle,管理连接生命周期与全部隧道。
//! 断线检测靠 handler 的 disconnected 回调 + mpsc 关闭(Handle 非 Clone 无法 clone 出来 poll)
use crate::forward::local::{bind_listener, spawn_local_forward};
use crate::forward::remote::{start_remote_forward, stop_remote_forward};
use crate::forward::socks::spawn_socks_forward;
use crate::known_hosts::KnownHosts;
use crate::model::{Forward, ForwardKind, ForwardStatus, Server, ServerStatus};
use crate::secrets::SecretStore;
use crate::ssh::client::{connect, ChannelOpener, Connection, HostKeyDecider, OpenChannelRequest, RemoteTarget};
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
    };
    tokio::spawn(actor.run());
    ActorHandle { tx }
}

impl Actor {
    fn emit_server(&self, status: ServerStatus, error: Option<String>) {
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
        let mut retry_at: Option<tokio::time::Instant> = None;
        let mut attempt: u32 = 0;
        loop {
            tokio::select! {
                cmd = self.rx.recv() => {
                    let Some(cmd) = cmd else { break };
                    match cmd {
                        ActorCommand::Shutdown => break,
                        ActorCommand::Connect => {
                            self.manual_disconnect = false;
                            if self.conn.is_none() {
                                match self.do_connect().await {
                                    Ok(()) => { attempt = 0; retry_at = None; }
                                    Err(e) => {
                                        self.emit_server(ServerStatus::Error, Some(e.to_string()));
                                        if self.auto_reconnect && !self.manual_disconnect {
                                            retry_at = Some(Self::next_retry(&mut attempt));
                                            self.emit_server(ServerStatus::Reconnecting, Some(e.to_string()));
                                        }
                                    }
                                }
                            }
                        }
                        ActorCommand::Disconnect => {
                            self.manual_disconnect = true;
                            retry_at = None;
                            self.teardown_conn().await;
                            self.emit_server(ServerStatus::Disconnected, None);
                        }
                        ActorCommand::StartForward(f) => self.start_forward(f).await,
                        ActorCommand::StopForward { forward_id } => self.stop_forward(&forward_id).await,
                        ActorCommand::SetAutoReconnect(v) => self.auto_reconnect = v,
                    }
                }
                req = self.open_rx.recv(), if self.conn.is_some() => {
                    let Some(req) = req else { continue };
                    let conn = self.conn.as_ref().unwrap();
                    let r = conn.handle
                        .channel_open_direct_tcpip(req.target_host, req.target_port, "127.0.0.1", 0)
                        .await
                        .map_err(CoreError::from);
                    let _ = req.respond.send(r);
                }
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
                        retry_at = Some(Self::next_retry(&mut attempt));
                    } else {
                        self.emit_server(ServerStatus::Disconnected, Some(reason));
                    }
                }
                () = async {
                    match retry_at {
                        Some(t) => tokio::time::sleep_until(t).await,
                        None => std::future::pending().await,
                    }
                } => {
                    retry_at = None;
                    match self.do_connect().await {
                        Ok(()) => attempt = 0,
                        Err(e) => {
                            self.emit_server(ServerStatus::Reconnecting, Some(e.to_string()));
                            retry_at = Some(Self::next_retry(&mut attempt));
                        }
                    }
                }
            }
        }
        // Shutdown:清理一切
        self.teardown_conn().await;
        self.forwards.clear();
    }

    fn next_retry(attempt: &mut u32) -> tokio::time::Instant {
        let delay = BACKOFF_INIT * 2u32.saturating_pow((*attempt).min(5));
        *attempt += 1;
        tokio::time::Instant::now() + delay.min(BACKOFF_MAX)
    }

    async fn do_connect(&mut self) -> Result<(), CoreError> {
        self.emit_server(ServerStatus::Connecting, None);
        let conn = connect(&self.server, self.secrets.clone(), self.known_hosts.clone(), self.decider.clone()).await;
        match conn {
            Ok(conn) => {
                self.conn = Some(conn);
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
        for (id, (forward, active)) in self.forwards.iter() {
            if matches!(active, ActiveForward::Remote) {
                match start_remote_forward(forward, &conn.handle, &conn.remote_forwards).await {
                    Ok(()) => self.emit_forward(id, ForwardStatus::Running, None),
                    Err(e) => self.emit_forward(id, ForwardStatus::Error, Some(e.to_string())),
                }
            }
        }
    }

    async fn start_forward(&mut self, forward: Forward) {
        self.emit_forward(&forward.id, ForwardStatus::Starting, None);
        // 联动规则:未连接先连接
        if self.conn.is_none() {
            if let Err(e) = self.do_connect().await {
                self.emit_forward(&forward.id, ForwardStatus::Error, Some(e.to_string()));
                return;
            }
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
                    start_remote_forward(&forward, &conn.handle, &conn.remote_forwards).await?;
                    Ok(ActiveForward::Remote)
                }
            }
        }
        .await;
        match result {
            Ok(active) => {
                self.forwards.insert(forward.id.clone(), (forward.clone(), active));
                self.emit_forward(&forward.id, ForwardStatus::Running, None);
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
                    let _ = stop_remote_forward(&forward, &conn.handle, &conn.remote_forwards).await;
                }
            }
        }
        self.emit_forward(forward_id, ForwardStatus::Stopped, None);
    }
}
```

借检查器提示：select 中 `self.open_rx.recv()` 与 `self.conn.as_mut()` 分支并存——`open_rx` 和 `conn` 是不同字段，借检查可通过（字段级拆分）。若报错，把 `open_rx`/`open_tx` 移出 Actor 结构体改为 `run(self)` 的局部变量、`self` 只留其余字段。

- [ ] **Step 5: 跑测试确认通过**

Run: `cargo test -p ssh-tunnel-core --test actor`
Expected: 4 passed（重连测试约需 1-2 秒退避）

- [ ] **Step 6: 跑全部 core 测试回归**

Run: `cargo test -p ssh-tunnel-core`
Expected: 全绿

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat: 服务器连接 actor（状态机、自动重连、隧道管理）"
```

### Task 11: SshManager —— CRUD 编排与状态快照

**Files:**
- Modify: `core/src/ssh/manager.rs`
- Test: `core/tests/manager.rs`

**Interfaces:**
- Consumes: Task 10 的 `spawn_actor`/`ActorHandle`/`TunnelEvent`，Task 2 的 `ConfigStore`/`AppConfig`
- Produces:
  - `#[derive(Debug, Clone, Serialize, Default)] pub struct StatusSnapshot { pub servers: HashMap<String, StatusEntry<ServerStatus>>, pub forwards: HashMap<String, StatusEntry<ForwardStatus>> }`；`StatusEntry<T> { status: T, error: Option<String> }`（定义在 `ssh/mod.rs`，泛型需 `Serialize + Clone`；T 无默认值时用 `Option<T>` 包装或给 entry 定义 `#[serde(default)]`——**简化：不用泛型**，定义 `ServerStatusEntry { status: ServerStatus, error: Option<String> }` 与 `ForwardStatusEntry { status: ForwardStatus, error: Option<String> }`）
  - `pub struct SshManager`，方法：
    - `pub fn new(store: ConfigStore, secrets: Arc<dyn SecretStore>, known_hosts: Arc<Mutex<KnownHosts>>, decider: HostKeyDecider) -> Result<Self, CoreError>`（load 配置）
    - `pub fn subscribe(&self) -> broadcast::Receiver<TunnelEvent>`
    - `pub async fn snapshot(&self) -> StatusSnapshot`
    - `pub async fn list_servers(&self) -> Vec<Server>` / `list_forwards(&self) -> Vec<Forward>` / `settings(&self) -> Settings`
    - `pub async fn upsert_server(&self, server: Server) -> Result<Server, CoreError>`（空 id 生成 uuid；已有连接时先 Shutdown 旧 actor）
    - `pub async fn delete_server(&self, id: &str) -> Result<(), CoreError>`（停 actor、删该服务器全部 secrets、级联删其 forwards）
    - `pub async fn upsert_forward(&self, forward: Forward) -> Result<Forward, CoreError>`（空 id 生成 uuid；若在运行则先 stop）
    - `pub async fn delete_forward(&self, id: &str) -> Result<(), CoreError>`
    - `pub async fn start_forward(&self, id: &str) / stop_forward(&self, id: &str) / connect_server(&self, id: &str) / disconnect_server(&self, id: &str) -> Result<(), CoreError>`
    - `pub async fn update_settings(&self, settings: Settings) -> Result<(), CoreError>`（落盘 + 向所有 actor 发 SetAutoReconnect）
    - `pub async fn start_auto_forwards(&self)`（启动所有 auto_start 的转发）
    - `pub async fn shutdown_all(&self)`
  - 每次 CRUD 后自动 `store.save`

- [ ] **Step 1: 写失败测试**

`core/tests/manager.rs`：
```rust
mod support;

use ssh_tunnel_core::config::ConfigStore;
use ssh_tunnel_core::known_hosts::KnownHosts;
use ssh_tunnel_core::model::*;
use ssh_tunnel_core::secrets::{MemorySecretStore, SecretKind, SecretStore};
use ssh_tunnel_core::ssh::client::HostKeyDecider;
use ssh_tunnel_core::ssh::manager::SshManager;
use ssh_tunnel_core::ssh::TunnelEvent;
use std::sync::Arc;
use tokio::sync::Mutex;
use support::*;

fn make_manager(dir: &std::path::Path) -> SshManager {
    let store = ConfigStore::new(dir.join("config.json"));
    let secrets = Arc::new(MemorySecretStore::new());
    let kh = Arc::new(Mutex::new(KnownHosts::new(dir.join("known_hosts"))));
    let decider: HostKeyDecider = Arc::new(|_| Box::pin(async { true }) as _);
    SshManager::new(store, secrets, kh, decider).unwrap()
}

fn server_with(id: &str, addr: std::net::SocketAddr) -> Server {
    Server {
        id: id.into(), name: "t".into(), host: addr.ip().to_string(),
        port: addr.port(), username: "u".into(), auth: AuthMethod::Password,
    }
}

#[tokio::test]
async fn crud_persists_to_disk() {
    let dir = tempfile::tempdir().unwrap();
    let mgr = make_manager(dir.path());
    let s = mgr.upsert_server(server_with("", "127.0.0.1:1".parse().unwrap())).await.unwrap();
    assert!(!s.id.is_empty(), "空 id 应生成 uuid");

    let f = Forward {
        id: String::new(), server_id: s.id.clone(), name: "mysql".into(),
        kind: ForwardKind::Local, bind_addr: "127.0.0.1".into(), bind_port: 3306,
        target_host: Some("db".into()), target_port: Some(3306), auto_start: false,
    };
    let f = mgr.upsert_forward(f).await.unwrap();

    // 重建 manager(模拟重启应用)验证持久化
    let mgr2 = make_manager(dir.path());
    assert_eq!(mgr2.list_servers().await.len(), 1);
    assert_eq!(mgr2.list_forwards().await.len(), 1);

    mgr2.delete_forward(&f.id).await.unwrap();
    assert!(mgr2.list_forwards().await.is_empty());
    mgr2.delete_server(&s.id).await.unwrap();
    assert!(mgr2.list_servers().await.is_empty());
}

#[tokio::test]
async fn delete_server_removes_secrets() {
    let dir = tempfile::tempdir().unwrap();
    let secrets = Arc::new(MemorySecretStore::new());
    let store = ConfigStore::new(dir.path().join("config.json"));
    let kh = Arc::new(Mutex::new(KnownHosts::new(dir.path().join("kh"))));
    let decider: HostKeyDecider = Arc::new(|_| Box::pin(async { true }) as _);
    let mgr = SshManager::new(store, secrets.clone(), kh, decider).unwrap();

    let s = mgr.upsert_server(server_with("", "127.0.0.1:1".parse().unwrap())).await.unwrap();
    secrets.set(&s.id, SecretKind::Password, "pw").unwrap();
    mgr.delete_server(&s.id).await.unwrap();
    assert_eq!(secrets.get(&s.id, SecretKind::Password).unwrap(), None);
}

#[tokio::test]
async fn start_forward_via_manager_end_to_end() {
    let echo = start_tcp_echo().await;
    let ssh = start_ssh_server(TestServerOpts { password: Some(TEST_PASSWORD), accept_keys: vec![] }).await;
    let dir = tempfile::tempdir().unwrap();
    let mgr = make_manager(dir.path());

    let s = mgr.upsert_server(server_with("", ssh.addr)).await.unwrap();
    // manager 用内存 secrets(测试注入);真实 app 是 KeyringStore
    // 注意:manager 构造时拿的是 make_manager 里新建的 secrets,这里需同一个实例——
    // 因此本测试改用下面的手工构造:
    drop(mgr);
    let secrets = Arc::new(MemorySecretStore::new());
    let store = ConfigStore::new(dir.path().join("config2.json"));
    let kh = Arc::new(Mutex::new(KnownHosts::new(dir.path().join("kh2"))));
    let decider: HostKeyDecider = Arc::new(|_| Box::pin(async { true }) as _);
    let mgr = SshManager::new(store, secrets.clone(), kh, decider).unwrap();

    let s = mgr.upsert_server(server_with(&s.id, ssh.addr)).await.unwrap();
    secrets.set(&s.id, SecretKind::Password, TEST_PASSWORD).unwrap();

    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let p = l.local_addr().unwrap().port();
        drop(l);
        p
    };
    let f = mgr.upsert_forward(Forward {
        id: String::new(), server_id: s.id.clone(), name: "echo".into(),
        kind: ForwardKind::Local, bind_addr: "127.0.0.1".into(), bind_port: port,
        target_host: Some(echo.ip().to_string()), target_port: Some(echo.port()),
        auto_start: false,
    }).await.unwrap();

    let mut rx = mgr.subscribe();
    mgr.start_forward(&f.id).await.unwrap();

    // 等 Running 事件
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        let ev = tokio::time::timeout(deadline - std::time::Instant::now(), rx.recv()).await.unwrap().unwrap();
        if matches!(ev, TunnelEvent::ForwardStatus { status: ForwardStatus::Running, .. }) { break; }
    }

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut client = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    client.write_all(b"mgr").await.unwrap();
    let mut buf = vec![0u8; 3];
    client.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"mgr");

    // 快照应反映运行状态
    let snap = mgr.snapshot().await;
    assert_eq!(snap.forwards.get(&f.id).unwrap().status, ForwardStatus::Running);
    assert_eq!(snap.servers.get(&s.id).unwrap().status, ServerStatus::Connected);
    mgr.shutdown_all().await;
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p ssh-tunnel-core --test manager`
Expected: 编译失败

- [ ] **Step 3: 实现 ssh/mod.rs 快照类型（追加）**

```rust
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize)]
pub struct ServerStatusEntry {
    pub status: ServerStatus,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ForwardStatusEntry {
    pub status: ForwardStatus,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct StatusSnapshot {
    pub servers: HashMap<String, ServerStatusEntry>,
    pub forwards: HashMap<String, ForwardStatusEntry>,
}
```

- [ ] **Step 4: 实现 ssh/manager.rs**

```rust
//! CRUD 编排:配置落盘、secrets 清理、actor 生命周期、状态快照维护
use crate::config::{AppConfig, ConfigStore};
use crate::known_hosts::KnownHosts;
use crate::model::{Forward, ForwardStatus, Server, ServerStatus, Settings};
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
            while let Ok(ev) = rx.recv().await {
                let mut snap = snap.write().await;
                match ev {
                    TunnelEvent::ServerStatus { server_id, status, error } => {
                        snap.servers.insert(server_id, ServerStatusEntry { status, error });
                    }
                    TunnelEvent::ForwardStatus { forward_id, status, error, .. } => {
                        snap.forwards.insert(forward_id, ForwardStatusEntry { status, error });
                    }
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
        {
            let mut cfg = self.config.write().await;
            if !cfg.servers.iter().any(|s| s.id == id) {
                return Err(CoreError::ServerNotFound(id.to_string()));
            }
            cfg.servers.retain(|s| s.id != id);
            cfg.forwards.retain(|f| f.server_id != id);
        }
        for kind in [SecretKind::Password, SecretKind::Key, SecretKind::KeyPassphrase] {
            let _ = self.secrets.delete(id, kind);
        }
        self.snapshot.write().await.servers.remove(id);
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
            actor.send(ActorCommand::StopForward { forward_id: id.to_string() })?;
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
```

- [ ] **Step 5: 跑测试确认通过**

Run: `cargo test -p ssh-tunnel-core`
Expected: 全部通过

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat: SshManager（CRUD 编排、状态快照、自动启动）"
```

### Task 12: Tauri 壳 —— commands、日志、host key 弹窗回路

**Files:**
- Create: `src-tauri/Cargo.toml`、`src-tauri/build.rs`、`src-tauri/tauri.conf.json`、`src-tauri/capabilities/default.json`
- Create: `src-tauri/src/main.rs`、`src-tauri/src/commands.rs`、`src-tauri/src/logging.rs`
- Create: `package.json`、`vite.config.ts`、`tsconfig.json`、`index.html`、`src/main.ts`、`src/App.vue`（最小占位，Task 14/15 填实）

**Interfaces:**
- Consumes: `SshManager` 全部公开方法、`StatusSnapshot`
- Produces（前端 Task 14 依赖这些签名）:
  - commands（参数即下表；错误一律 `Result<T, String>`）：
    - `list_servers() -> Vec<Server>`
    - `upsert_server(input: UpsertServerInput) -> Server`，`UpsertServerInput { server: Server, password: Option<String>, key_data: Option<String>, key_passphrase: Option<String> }`（Some = 写入钥匙串；None = 保持不变）
    - `delete_server(id: String)`
    - `list_forwards() -> Vec<Forward>` / `upsert_forward(forward: Forward) -> Forward` / `delete_forward(id: String)`
    - `start_forward(id: String)` / `stop_forward(id: String)` / `connect_server(id: String)` / `disconnect_server(id: String)`
    - `get_snapshot() -> StatusSnapshot` / `get_settings() -> Settings` / `save_settings(settings: Settings)`
    - `get_logs() -> Vec<LogEntry>` / `respond_host_key(prompt_id: String, trust: bool)`
  - Tauri events：`tunnel-event`（TunnelEvent）、`log`（LogEntry）、`host-key-prompt`（`{ prompt_id, host, port, fingerprint, is_mismatch }`）
  - `LogEntry { timestamp: String, level: String, message: String }`

- [ ] **Step 1: 安装系统依赖（需要用户执行）**

Tauri 在 Linux 编译需要 webkit 开发包。sudo 需要交互密码，请用户在对话中执行：

```
! sudo apt install -y libwebkit2gtk-4.1-dev libappindicator3-dev
```

（无外网时按 Global Constraints 加代理前缀。）Windows/macOS 无此步骤。

- [ ] **Step 2: 建 src-tauri crate 骨架**

`src-tauri/Cargo.toml`：
```toml
[package]
name = "ssh-tunnel-app"
version = "0.1.0"
edition.workspace = true

[build-dependencies]
tauri-build = { version = "2", features = [] }

[dependencies]
ssh-tunnel-core = { path = "../core" }
tauri = { version = "2", features = [] }
tauri-plugin-autostart = "2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["full"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["fmt", "registry"] }
tracing-appender = "0.2"
chrono = "0.4"
uuid = { version = "1", features = ["v4"] }

[features]
# tauri 的 dev 模式热重载默认不开;自定义不需要
```

`src-tauri/build.rs`：
```rust
fn main() {
    tauri_build::build()
}
```

`src-tauri/tauri.conf.json`：
```json
{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "ssh-tunnel",
  "version": "0.1.0",
  "identifier": "com.sshtunnel.app",
  "build": {
    "beforeDevCommand": "pnpm dev",
    "devUrl": "http://localhost:1420",
    "beforeBuildCommand": "pnpm build",
    "frontendDist": "../dist"
  },
  "app": {
    "windows": [
      {
        "label": "main",
        "title": "SSH Tunnel",
        "width": 900,
        "height": 620,
        "minWidth": 720,
        "minHeight": 480
      }
    ],
    "security": { "csp": null }
  },
  "bundle": {
    "active": true,
    "targets": "all",
    "icon": []
  }
}
```
（`icon: []`：开发期不打包图标；发布打包前用 `pnpm tauri icon` 生成后补上路径。）

`src-tauri/capabilities/default.json`：
```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "default",
  "description": "main window",
  "windows": ["main"],
  "permissions": ["core:default", "core:event:default", "core:window:default", "core:tray:default", "core:menu:default", "autostart:default"]
}
```

- [ ] **Step 3: 前端最小骨架（保证 `pnpm dev`/`pnpm build` 可跑）**

`package.json`：
```json
{
  "name": "ssh-tunnel",
  "private": true,
  "version": "0.1.0",
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "vue-tsc --noEmit && vite build",
    "test": "vitest run",
    "type-check": "vue-tsc --noEmit",
    "tauri": "tauri"
  }
}
```

然后装依赖（网络失败时按 Global Constraints 加代理）：
```bash
pnpm add vue@^3 pinia element-plus @tauri-apps/api@^2 @tauri-apps/plugin-autostart@^2
pnpm add -D vite @vitejs/plugin-vue typescript vue-tsc vitest @vue/test-utils jsdom @tauri-apps/cli@^2
```

`vite.config.ts`：
```ts
/// <reference types="vitest/config" />
import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

export default defineConfig({
  plugins: [vue()],
  server: { port: 1420, strictPort: true },
  clearScreen: false,
  test: {
    environment: 'jsdom',
    globals: true,
  },
})
```

`tsconfig.json`：
```json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ESNext",
    "moduleResolution": "bundler",
    "strict": true,
    "jsx": "preserve",
    "types": ["vite/client", "vitest/globals"],
    "lib": ["ES2022", "DOM"],
    "skipLibCheck": true,
    "noEmit": true
  },
  "include": ["src/**/*.ts", "src/**/*.vue", "vite.config.ts"]
}
```

`index.html`：
```html
<!doctype html>
<html lang="zh-CN">
  <head><meta charset="UTF-8" /><title>SSH Tunnel</title></head>
  <body><div id="app"></div><script type="module" src="/src/main.ts"></script></body>
</html>
```

`src/main.ts`、`src/App.vue` 最小占位：
```ts
// src/main.ts
import { createApp } from 'vue'
import App from './App.vue'
createApp(App).mount('#app')
```
```vue
<!-- src/App.vue:Task 15 实现完整界面 -->
<template><div>SSH Tunnel</div></template>
```

- [ ] **Step 4: 实现 logging.rs**

```rust
//! 日志三去向:stdout、滚动文件、前端事件
use serde::Serialize;
use ssh_tunnel_core::paths;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
use tracing_subscriber::Registry;

#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    pub timestamp: String,
    pub level: String,
    pub message: String,
}

const MAX_LOGS: usize = 500;

#[derive(Clone, Default)]
pub struct LogBuffer {
    inner: Arc<Mutex<VecDeque<LogEntry>>>,
}

impl LogBuffer {
    pub fn push(&self, entry: LogEntry) {
        let mut logs = self.inner.lock().unwrap();
        if logs.len() >= MAX_LOGS {
            logs.pop_front();
        }
        logs.push_back(entry);
    }
    pub fn snapshot(&self) -> Vec<LogEntry> {
        self.inner.lock().unwrap().iter().cloned().collect()
    }
}

struct BufferLayer {
    buffer: LogBuffer,
}

impl<S: tracing::Subscriber> Layer<S> for BufferLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let mut message = String::new();
        let mut visitor = MessageVisitor(&mut message);
        event.record(&mut visitor);
        self.buffer.push(LogEntry {
            timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
            level: event.metadata().level().to_string(),
            message,
        });
    }
}

struct MessageVisitor<'a>(&'a mut String);

impl tracing::field::Visit for MessageVisitor<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            *self.0 = format!("{value:?}");
        }
    }
}

pub fn init_logging() -> LogBuffer {
    let buffer = LogBuffer::default();
    let log_dir = paths::config_dir().join("logs");
    std::fs::create_dir_all(&log_dir).ok();
    let file = tracing_appender::rolling::daily(&log_dir, "ssh-tunnel.log");
    tracing::subscriber::set_global_default(
        Registry::default()
            .with(BufferLayer { buffer: buffer.clone() })
            .with(tracing_subscriber::fmt::layer().with_writer(std::io::stdout))
            .with(tracing_subscriber::fmt::layer().with_ansi(false).with_writer(file)),
    )
    .expect("初始化日志失败");
    buffer
}
```

- [ ] **Step 5: 实现 commands.rs**

```rust
use crate::logging::LogEntry;
use crate::{tray, AppState};
use ssh_tunnel_core::model::{Forward, Server, Settings};
use ssh_tunnel_core::secrets::SecretKind;
use ssh_tunnel_core::ssh::StatusSnapshot;
use tauri::{AppHandle, State};

#[derive(Debug, serde::Deserialize)]
pub struct UpsertServerInput {
    pub server: Server,
    /// Some = 写入钥匙串;None = 保持已有值不变
    pub password: Option<String>,
    pub key_data: Option<String>,
    pub key_passphrase: Option<String>,
}

fn err(e: ssh_tunnel_core::CoreError) -> String {
    e.to_string()
}

/// 配置类变更后刷新托盘缓存与菜单(状态类变更走事件回路,见 main.rs)
async fn after_config_change(app: &AppHandle) {
    tray::refresh_cache(app).await;
    tray::refresh_tray(app);
}

#[tauri::command]
pub async fn list_servers(state: State<'_, AppState>) -> Result<Vec<Server>, String> {
    Ok(state.manager.list_servers().await)
}

#[tauri::command]
pub async fn upsert_server(input: UpsertServerInput, app: AppHandle, state: State<'_, AppState>) -> Result<Server, String> {
    // 认证方式变更时旧凭据作废,先清再写
    let old = state.manager.list_servers().await.into_iter().find(|s| s.id == input.server.id);
    if let Some(old) = old {
        if old.auth != input.server.auth {
            for kind in [SecretKind::Password, SecretKind::Key, SecretKind::KeyPassphrase] {
                let _ = state.manager.secrets().delete(&input.server.id, kind);
            }
        }
    }
    let saved = state.manager.upsert_server(input.server).await.map_err(err)?;
    if let Some(v) = input.password {
        state.manager.secrets().set(&saved.id, SecretKind::Password, &v).map_err(err)?;
    }
    if let Some(v) = input.key_data {
        state.manager.secrets().set(&saved.id, SecretKind::Key, &v).map_err(err)?;
    }
    if let Some(v) = input.key_passphrase {
        state.manager.secrets().set(&saved.id, SecretKind::KeyPassphrase, &v).map_err(err)?;
    }
    after_config_change(&app).await;
    Ok(saved)
}

#[tauri::command]
pub async fn delete_server(id: String, app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    state.manager.delete_server(&id).await.map_err(err)?;
    after_config_change(&app).await;
    Ok(())
}

#[tauri::command]
pub async fn list_forwards(state: State<'_, AppState>) -> Result<Vec<Forward>, String> {
    Ok(state.manager.list_forwards().await)
}

#[tauri::command]
pub async fn upsert_forward(forward: Forward, app: AppHandle, state: State<'_, AppState>) -> Result<Forward, String> {
    let saved = state.manager.upsert_forward(forward).await.map_err(err)?;
    after_config_change(&app).await;
    Ok(saved)
}

#[tauri::command]
pub async fn delete_forward(id: String, app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    state.manager.delete_forward(&id).await.map_err(err)?;
    after_config_change(&app).await;
    Ok(())
}

#[tauri::command]
pub async fn start_forward(id: String, state: State<'_, AppState>) -> Result<(), String> {
    state.manager.start_forward(&id).await.map_err(err)
}

#[tauri::command]
pub async fn stop_forward(id: String, state: State<'_, AppState>) -> Result<(), String> {
    state.manager.stop_forward(&id).await.map_err(err)
}

#[tauri::command]
pub async fn connect_server(id: String, state: State<'_, AppState>) -> Result<(), String> {
    state.manager.connect_server(&id).await.map_err(err)
}

#[tauri::command]
pub async fn disconnect_server(id: String, state: State<'_, AppState>) -> Result<(), String> {
    state.manager.disconnect_server(&id).await.map_err(err)
}

#[tauri::command]
pub async fn get_snapshot(state: State<'_, AppState>) -> Result<StatusSnapshot, String> {
    Ok(state.manager.snapshot().await)
}

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> Result<Settings, String> {
    Ok(state.manager.settings().await)
}

#[tauri::command]
pub async fn save_settings(settings: Settings, app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    state.manager.update_settings(settings.clone()).await.map_err(err)?;
    *state.settings_cache.write().unwrap() = settings.clone();
    // 开机自启跟随设置
    use tauri_plugin_autostart::ManagerExt;
    let autostart = app.autolaunch();
    let _ = if settings.launch_at_login { autostart.enable() } else { autostart.disable() };
    Ok(())
}

#[tauri::command]
pub async fn get_logs(state: State<'_, AppState>) -> Result<Vec<LogEntry>, String> {
    Ok(state.logs.snapshot())
}

#[tauri::command]
pub async fn respond_host_key(prompt_id: String, trust: bool, state: State<'_, AppState>) -> Result<(), String> {
    let sender = state.pending_host_keys.lock().await.remove(&prompt_id);
    if let Some(tx) = sender {
        let _ = tx.send(trust);
    }
    Ok(())
}
```

注意：`state.manager.secrets()` 需要在 SshManager 上加一个访问器（Task 11 补一行）：

```rust
pub fn secrets(&self) -> &Arc<dyn SecretStore> {
    &self.secrets
}
```

- [ ] **Step 6: 实现 main.rs**

```rust
mod commands;
mod logging;
mod tray;

use ssh_tunnel_core::config::ConfigStore;
use ssh_tunnel_core::known_hosts::KnownHosts;
use ssh_tunnel_core::model::{Forward, Server, Settings};
use ssh_tunnel_core::paths;
use ssh_tunnel_core::secrets::KeyringStore;
use ssh_tunnel_core::ssh::client::{HostKeyDecider, HostKeyInfo};
use ssh_tunnel_core::ssh::manager::SshManager;
use ssh_tunnel_core::ssh::StatusSnapshot;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tauri::{Emitter, Manager};
use tokio::sync::{oneshot, Mutex};

/// 托盘的同步数据源:托盘菜单构建发生在任意回调上下文,
/// 不能 await(在 runtime 线程里 block_on 会 panic),故用 RwLock 缓存,
/// 由事件任务与 CRUD command 主动刷新
#[derive(Default)]
pub struct TrayCache {
    pub servers: Vec<Server>,
    pub forwards: Vec<Forward>,
    pub snapshot: StatusSnapshot,
}

pub struct AppState {
    pub manager: Arc<SshManager>,
    pub pending_host_keys: Arc<Mutex<HashMap<String, oneshot::Sender<bool>>>>,
    pub settings_cache: RwLock<Settings>,
    pub tray_cache: RwLock<TrayCache>,
    pub logs: logging::LogBuffer,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let logs = logging::init_logging();

    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![]),
        ))
        .setup(|app| {
            let dir = paths::config_dir();
            std::fs::create_dir_all(&dir)?;
            let store = ConfigStore::new(dir.join("config.json"));
            let known_hosts = Arc::new(Mutex::new(KnownHosts::new(dir.join("known_hosts"))));
            let pending: Arc<Mutex<HashMap<String, oneshot::Sender<bool>>>> = Arc::new(Mutex::new(HashMap::new()));

            // host key 决策回路:core 回调 → tauri event → 前端弹窗 → respond_host_key command。
            // pending map 与 AppState 共享同一个 Arc,respond_host_key 才能找到回调
            let app_handle = app.handle().clone();
            let pending_for_decider = pending.clone();
            let decider: HostKeyDecider = Arc::new(move |info: HostKeyInfo| {
                let app = app_handle.clone();
                let pending = pending_for_decider.clone();
                Box::pin(async move {
                    let (tx, rx) = oneshot::channel();
                    let prompt_id = uuid::Uuid::new_v4().to_string();
                    pending.lock().await.insert(prompt_id.clone(), tx);
                    let payload = serde_json::json!({
                        "prompt_id": prompt_id,
                        "host": info.host,
                        "port": info.port,
                        "fingerprint": info.fingerprint,
                        "is_mismatch": info.is_mismatch,
                    });
                    if app.emit("host-key-prompt", payload).is_err() {
                        return false;
                    }
                    rx.await.unwrap_or(false)
                })
            });

            let manager = SshManager::new(store, Arc::new(KeyringStore::new()), known_hosts, decider)
                .map_err(|e| format!("加载配置失败: {e}"))?;
            let manager = Arc::new(manager);
            // setup 是同步上下文,此处 block_on 安全(不在 runtime worker 内)
            let settings = tauri::async_runtime::block_on(manager.settings());

            app.manage(AppState {
                manager: manager.clone(),
                pending_host_keys: pending,
                settings_cache: RwLock::new(settings),
                tray_cache: RwLock::new(TrayCache::default()),
                logs,
            });

            // core 事件 → 前端 + 托盘缓存 + 托盘菜单
            let mut rx = manager.subscribe();
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                tray::refresh_cache(&app_handle).await;
                tray::refresh_tray(&app_handle);
                while let Ok(ev) = rx.recv().await {
                    let _ = app_handle.emit("tunnel-event", &ev);
                    tray::refresh_cache(&app_handle).await;
                    tray::refresh_tray(&app_handle);
                }
            });

            tray::build_tray(app)?;

            // 恢复 auto_start 转发
            tauri::async_runtime::spawn(async move {
                manager.start_auto_forwards().await;
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_servers,
            commands::upsert_server,
            commands::delete_server,
            commands::list_forwards,
            commands::upsert_forward,
            commands::delete_forward,
            commands::start_forward,
            commands::stop_forward,
            commands::connect_server,
            commands::disconnect_server,
            commands::get_snapshot,
            commands::get_settings,
            commands::save_settings,
            commands::get_logs,
            commands::respond_host_key,
        ])
        .on_window_event(|window, event| {
            // 关闭主窗口 = 最小化到托盘(可在设置中关闭此行为)
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let minimize = window
                    .state::<AppState>()
                    .settings_cache
                    .read()
                    .map(|s| s.minimize_to_tray)
                    .unwrap_or(true);
                if minimize {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("运行 tauri 应用失败");
}

fn main() {
    run()
}
```

（按 Tauri 2 惯例也可拆 `lib.rs` + `main.rs` 调 `run()`；单文件 main.rs 亦可，保持简单。）

- [ ] **Step 7: 验证编译**

Run: `cargo build -p ssh-tunnel-app && pnpm build`
Expected: 编译通过（tray.rs 为空占位 `pub fn build_tray(...) -> tauri::Result<()> { Ok(()) }` 与 `pub fn refresh_tray(_) {}`，Task 13 填实）

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "feat: Tauri 壳（commands、日志、host key 弹窗回路）"
```

---

### Task 13: 系统托盘

**Files:**
- Create: `src-tauri/src/tray.rs`
- Modify: `src-tauri/src/main.rs`（若有接线遗漏）

**Interfaces:**
- Consumes: `AppState.manager.snapshot()/list_servers()/list_forwards()`
- Produces:
  - `pub fn build_tray(app: &mut tauri::App) -> tauri::Result<()>`
  - `pub fn refresh_tray(app: &AppHandle)`（从最新快照重建菜单 + 更新图标；事件驱动调用）
- 菜单项 id 约定：`fwd:<forwardId>`（勾选启停）、`add:<serverId>`、服务器标题 `srv:<serverId>`（disabled）、`show`、`quit`

- [ ] **Step 1: 实现 tray.rs**

```rust
//! 托盘:三态图标 + 按服务器分组的转发启停菜单。
//! 菜单构建只能同步读 tray_cache(回调上下文里不能 await);
//! 数据由 refresh_cache 在事件任务/command 里异步刷新
use crate::{AppState, TrayCache};
use ssh_tunnel_core::model::{ForwardKind, ForwardStatus, ServerStatus};
use tauri::image::Image;
use tauri::menu::{CheckMenuItemBuilder, Menu, MenuBuilder, MenuItemBuilder, PredefinedMenuItem, SubmenuBuilder};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager};

const TRAY_ID: &str = "main";

/// 纯代码生成圆形图标(免图标资源文件):灰=无活动,绿=正常,红=有错误
fn circle_icon(rgba: [u8; 4]) -> Image<'static> {
    const S: u32 = 32;
    let mut data = vec![0u8; (S * S * 4) as usize];
    let c = S as f32 / 2.0;
    let r = c - 2.0;
    for y in 0..S {
        for x in 0..S {
            let dx = x as f32 - c + 0.5;
            let dy = y as f32 - c + 0.5;
            if dx * dx + dy * dy <= r * r {
                let i = ((y * S + x) * 4) as usize;
                data[i..i + 4].copy_from_slice(&rgba);
            }
        }
    }
    Image::new_owned(data, S, S)
}

const GREY: [u8; 4] = [158, 158, 158, 255];
const GREEN: [u8; 4] = [76, 175, 80, 255];
const RED: [u8; 4] = [244, 67, 54, 255];

/// 从 manager 拉最新数据进托盘缓存(异步;在事件任务或 command 里调用)
pub async fn refresh_cache(app: &AppHandle) {
    let state = app.state::<AppState>();
    let servers = state.manager.list_servers().await;
    let forwards = state.manager.list_forwards().await;
    let snapshot = state.manager.snapshot().await;
    *state.tray_cache.write().unwrap() = TrayCache { servers, forwards, snapshot };
}

fn forward_label(f: &ssh_tunnel_core::model::Forward) -> String {
    match f.kind {
        ForwardKind::Local => format!("{} (本地 :{} → {}:{})", f.name, f.bind_port, f.target_host.as_deref().unwrap_or(""), f.target_port.unwrap_or(0)),
        ForwardKind::Remote => format!("{} (远程 :{} → {}:{})", f.name, f.bind_port, f.target_host.as_deref().unwrap_or(""), f.target_port.unwrap_or(0)),
        ForwardKind::Dynamic => format!("{} (SOCKS5 :{})", f.name, f.bind_port),
    }
}

fn build_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let state = app.state::<AppState>();
    let cache = state.tray_cache.read().unwrap();

    let mut builder = MenuBuilder::new(app);
    if cache.servers.is_empty() {
        builder = builder.item(&MenuItemBuilder::with_id("empty", "暂无服务器,点击「显示主窗口」添加").enabled(false).build(app)?);
    }
    for server in &cache.servers {
        let status = cache.snapshot.servers.get(&server.id).map(|s| s.status);
        let status_text = match status {
            Some(ServerStatus::Connected) => "已连接",
            Some(ServerStatus::Connecting) => "连接中…",
            Some(ServerStatus::Reconnecting) => "重连中…",
            Some(ServerStatus::Error) => "错误",
            _ => "未连接",
        };
        let mut sub = SubmenuBuilder::new(app, format!("{} ({})", server.name, status_text));
        let server_forwards: Vec<_> = cache.forwards.iter().filter(|f| f.server_id == server.id).collect();
        if server_forwards.is_empty() {
            sub = sub.item(&MenuItemBuilder::with_id(format!("none:{}", server.id), "暂无转发").enabled(false).build(app)?);
        }
        for f in server_forwards {
            let running = matches!(cache.snapshot.forwards.get(&f.id).map(|s| s.status), Some(ForwardStatus::Running));
            sub = sub.item(&CheckMenuItemBuilder::with_id(format!("fwd:{}", f.id), forward_label(f)).checked(running).build(app)?);
        }
        sub = sub.separator().item(&MenuItemBuilder::with_id(format!("add:{}", server.id), "添加转发…").build(app)?);
        builder = builder.item(&sub.build()?);
    }
    builder = builder
        .item(&PredefinedMenuItem::separator(app)?)
        .item(&MenuItemBuilder::with_id("show", "显示主窗口").build(app)?)
        .item(&MenuItemBuilder::with_id("quit", "退出").build(app)?);
    builder.build()
}

fn overall_icon(app: &AppHandle) -> Image<'static> {
    let state = app.state::<AppState>();
    let cache = state.tray_cache.read().unwrap();
    let has_error = cache.snapshot.servers.values().any(|s| s.status == ServerStatus::Error || s.status == ServerStatus::Reconnecting)
        || cache.snapshot.forwards.values().any(|s| s.status == ForwardStatus::Error);
    let has_running = cache.snapshot.forwards.values().any(|s| s.status == ForwardStatus::Running);
    let color = if has_error { RED } else if has_running { GREEN } else { GREY };
    circle_icon(color)
}

pub fn refresh_tray(app: &AppHandle) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else { return };
    if let Ok(menu) = build_menu(app) {
        let _ = tray.set_menu(Some(menu));
    }
    let _ = tray.set_icon(Some(overall_icon(app)));
}

pub fn build_tray(app: &mut tauri::App) -> tauri::Result<()> {
    let menu = build_menu(&app.handle())?;
    TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("SSH Tunnel")
        .icon(circle_icon(GREY))
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            let id = event.id().0.as_str();
            if let Some(fwd_id) = id.strip_prefix("fwd:") {
                toggle_forward(app, fwd_id);
            } else if let Some(server_id) = id.strip_prefix("add:") {
                show_window(app);
                let _ = app.emit("navigate", serde_json::json!({ "view": "add-forward", "server_id": server_id }));
            } else if id == "show" {
                show_window(app);
            } else if id == "quit" {
                let state = app.state::<AppState>();
                let manager = state.manager.clone();
                // 先优雅关停所有连接再退出
                tauri::async_runtime::spawn(async move {
                    manager.shutdown_all().await;
                    std::process::exit(0);
                });
            }
        })
        .build(app)?;
    Ok(())
}

fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn toggle_forward(app: &AppHandle, forward_id: &str) {
    let state = app.state::<AppState>();
    let manager = state.manager.clone();
    let forward_id = forward_id.to_string();
    let running = {
        let cache = state.tray_cache.read().unwrap();
        matches!(
            cache.snapshot.forwards.get(&forward_id).map(|s| s.status),
            Some(ForwardStatus::Running) | Some(ForwardStatus::Starting)
        )
    };
    tauri::async_runtime::spawn(async move {
        let result = if running {
            manager.stop_forward(&forward_id).await
        } else {
            manager.start_forward(&forward_id).await
        };
        if let Err(e) = result {
            tracing::error!("切换转发失败: {e}");
        }
    });
}
```

`lib.rs` 无需改；确认 `main.rs` 中 `tray::refresh_tray(&app_handle)` 已在事件循环里调用（Task 12 Step 6 已含）。

- [ ] **Step 2: 验证编译**

Run: `cargo build -p ssh-tunnel-app`
Expected: 编译通过。托盘交互（点击、菜单、图标变色）无单元测试，列为手动验证项，记入 Task 15 的总验证清单。

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "feat: 系统托盘（三态图标、转发启停菜单）"
```

### Task 14: 前端 api 封装 + Pinia stores

**Files:**
- Create: `src/types.ts`、`src/api.ts`
- Create: `src/stores/servers.ts`、`src/stores/forwards.ts`、`src/stores/logs.ts`
- Test: `src/stores/__tests__/forwards.test.ts`、`src/stores/__tests__/servers.test.ts`

**Interfaces:**
- Consumes: Task 12 的 commands 与 events
- Produces（Task 15 组件依赖）:
  - `src/types.ts`：`Server`、`AuthMethod`（`{ type: 'password' } | { type: 'key_file', path: string } | { type: 'key_data' }`）、`Forward`、`ForwardKind`、`Settings`、`ServerStatus`、`ForwardStatus`、`TunnelEvent`、`LogEntry`、`StatusSnapshot`（字段与 Rust serde 输出一致，snake_case）
  - `src/api.ts`：`api.listServers()`、`api.upsertServer(input)`、`api.deleteServer(id)`、`api.listForwards()`、`api.upsertForward(f)`、`api.deleteForward(id)`、`api.startForward(id)`、`api.stopForward(id)`、`api.connectServer(id)`、`api.disconnectServer(id)`、`api.getSnapshot()`、`api.getSettings()`、`api.saveSettings(s)`、`api.getLogs()`、`api.respondHostKey(promptId, trust)`、`onTunnelEvent(cb)`、`onLog(cb)`、`onHostKeyPrompt(cb)`、`onNavigate(cb)`（各自返回 unlisten 函数）
  - `useServersStore`：`{ servers, selectedId, serverStatus: Record<string, {status, error?}>, hostKeyPrompt, load(), select(id), save(input), remove(id), connect(id), disconnect(id) }`
  - `useForwardsStore`：`{ forwards, forwardStatus: Record<string, {status, error?}>, load(), forwardsOf(serverId), save(f), remove(id), toggle(id) }`
  - `useLogsStore`：`{ entries, load(), clear() }`
  - `bindTunnelEvents()`（在 App.vue onMounted 调一次）：把 `tunnel-event` 写入两个 store 的 status 表，把 `host-key-prompt` 写入 servers store

- [ ] **Step 1: 写 src/types.ts**

```ts
export type AuthMethod =
  | { type: 'password' }
  | { type: 'key_file'; path: string }
  | { type: 'key_data' }

export interface Server {
  id: string
  name: string
  host: string
  port: number
  username: string
  auth: AuthMethod
}

export type ForwardKind = 'local' | 'remote' | 'dynamic'

export interface Forward {
  id: string
  server_id: string
  name: string
  kind: ForwardKind
  bind_addr: string
  bind_port: number
  target_host: string | null
  target_port: number | null
  auto_start: boolean
}

export interface Settings {
  auto_reconnect: boolean
  minimize_to_tray: boolean
  launch_at_login: boolean
}

export type ServerStatus = 'disconnected' | 'connecting' | 'connected' | 'reconnecting' | 'error'
export type ForwardStatus = 'stopped' | 'starting' | 'running' | 'error'

export interface StatusEntry<T> {
  status: T
  error: string | null
}

export interface StatusSnapshot {
  servers: Record<string, StatusEntry<ServerStatus>>
  forwards: Record<string, StatusEntry<ForwardStatus>>
}

export type TunnelEvent =
  | { type: 'server_status'; server_id: string; status: ServerStatus; error: string | null }
  | { type: 'forward_status'; forward_id: string; server_id: string; status: ForwardStatus; error: string | null }

export interface LogEntry {
  timestamp: string
  level: string
  message: string
}

export interface HostKeyPrompt {
  prompt_id: string
  host: string
  port: number
  fingerprint: string
  is_mismatch: boolean
}

export interface UpsertServerInput {
  server: Server
  password?: string | null
  key_data?: string | null
  key_passphrase?: string | null
}
```

- [ ] **Step 2: 写 src/api.ts**

```ts
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import type {
  Forward, HostKeyPrompt, LogEntry, Server, Settings, StatusSnapshot, TunnelEvent, UpsertServerInput,
} from './types'

export const api = {
  listServers: () => invoke<Server[]>('list_servers'),
  upsertServer: (input: UpsertServerInput) => invoke<Server>('upsert_server', { input }),
  deleteServer: (id: string) => invoke<void>('delete_server', { id }),
  listForwards: () => invoke<Forward[]>('list_forwards'),
  upsertForward: (forward: Forward) => invoke<Forward>('upsert_forward', { forward }),
  deleteForward: (id: string) => invoke<void>('delete_forward', { id }),
  startForward: (id: string) => invoke<void>('start_forward', { id }),
  stopForward: (id: string) => invoke<void>('stop_forward', { id }),
  connectServer: (id: string) => invoke<void>('connect_server', { id }),
  disconnectServer: (id: string) => invoke<void>('disconnect_server', { id }),
  getSnapshot: () => invoke<StatusSnapshot>('get_snapshot'),
  getSettings: () => invoke<Settings>('get_settings'),
  saveSettings: (settings: Settings) => invoke<void>('save_settings', { settings }),
  getLogs: () => invoke<LogEntry[]>('get_logs'),
  respondHostKey: (promptId: string, trust: boolean) =>
    invoke<void>('respond_host_key', { promptId, trust }),
}

export const onTunnelEvent = (cb: (ev: TunnelEvent) => void) =>
  listen<TunnelEvent>('tunnel-event', (e) => cb(e.payload))
export const onLog = (cb: (entry: LogEntry) => void) =>
  listen<LogEntry>('log', (e) => cb(e.payload))
export const onHostKeyPrompt = (cb: (p: HostKeyPrompt) => void) =>
  listen<HostKeyPrompt>('host-key-prompt', (e) => cb(e.payload))
export const onNavigate = (cb: (nav: { view: string; server_id?: string }) => void) =>
  listen<{ view: string; server_id?: string }>('navigate', (e) => cb(e.payload))
```

- [ ] **Step 3: 写失败测试（mock Tauri）**

`src/stores/__tests__/mock-tauri.ts`（两个测试文件共用）：
```ts
import { vi } from 'vitest'
import type { TunnelEvent } from '../../types'

type TunnelHandler = (ev: TunnelEvent) => void

let tunnelHandler: TunnelHandler | null = null
export const invokeMock = vi.fn()

export function emitTunnel(ev: TunnelEvent) {
  tunnelHandler?.(ev)
}

export function installTauriMock() {
  vi.mock('@tauri-apps/api/core', () => ({
    invoke: (...args: unknown[]) => invokeMock(...args),
  }))
  vi.mock('@tauri-apps/api/event', () => ({
    listen: (name: string, handler: (e: { payload: unknown }) => void) => {
      if (name === 'tunnel-event') tunnelHandler = (ev) => handler({ payload: ev })
      return Promise.resolve(() => {})
    },
  }))
}
```

`src/stores/__tests__/forwards.test.ts`：
```ts
import { installTauriMock, invokeMock, emitTunnel } from './mock-tauri'
installTauriMock()

import { setActivePinia, createPinia } from 'pinia'
import { useForwardsStore, bindForwardsEvents } from '../forwards'
import type { Forward } from '../../types'

function fwd(over: Partial<Forward> = {}): Forward {
  return {
    id: 'f1', server_id: 's1', name: 'mysql', kind: 'local',
    bind_addr: '127.0.0.1', bind_port: 3306,
    target_host: 'db', target_port: 3306, auto_start: false,
    ...over,
  }
}

describe('forwards store', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    invokeMock.mockReset()
  })

  it('load 拉取转发与快照', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'list_forwards') return Promise.resolve([fwd()])
      if (cmd === 'get_snapshot') return Promise.resolve({ servers: {}, forwards: { f1: { status: 'running', error: null } } })
      return Promise.resolve(null)
    })
    const store = useForwardsStore()
    await store.load()
    expect(store.forwards).toHaveLength(1)
    expect(store.forwardStatus['f1'].status).toBe('running')
  })

  it('forwardsOf 按服务器过滤', async () => {
    invokeMock.mockResolvedValue([fwd(), fwd({ id: 'f2', server_id: 's2' })])
    const store = useForwardsStore()
    await store.load()
    expect(store.forwardsOf('s1').map((f) => f.id)).toEqual(['f1'])
  })

  it('toggle 停启判断:running → stop,其余 → start', async () => {
    invokeMock.mockResolvedValue([fwd()])
    const store = useForwardsStore()
    await store.load()
    await store.toggle('f1')
    expect(invokeMock).toHaveBeenLastCalledWith('start_forward', { id: 'f1' })

    store.forwardStatus['f1'] = { status: 'running', error: null }
    await store.toggle('f1')
    expect(invokeMock).toHaveBeenLastCalledWith('stop_forward', { id: 'f1' })
  })

  it('tunnel-event 更新状态表', async () => {
    invokeMock.mockResolvedValue([])
    const store = useForwardsStore()
    await store.load()
    bindForwardsEvents()
    emitTunnel({ type: 'forward_status', forward_id: 'f1', server_id: 's1', status: 'error', error: '本地端口 3306 被占用' })
    expect(store.forwardStatus['f1']).toEqual({ status: 'error', error: '本地端口 3306 被占用' })
  })
})
```

`src/stores/__tests__/servers.test.ts`：
```ts
import { installTauriMock, invokeMock } from './mock-tauri'
installTauriMock()

import { setActivePinia, createPinia } from 'pinia'
import { useServersStore } from '../servers'
import type { Server } from '../../types'

function srv(over: Partial<Server> = {}): Server {
  return { id: 's1', name: 'db', host: '10.0.0.2', port: 22, username: 'u', auth: { type: 'password' }, ...over }
}

describe('servers store', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    invokeMock.mockReset()
  })

  it('load 后自动选中第一台', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'list_servers') return Promise.resolve([srv()])
      if (cmd === 'get_snapshot') return Promise.resolve({ servers: {}, forwards: {} })
      return Promise.resolve(null)
    })
    const store = useServersStore()
    await store.load()
    expect(store.selectedId).toBe('s1')
  })

  it('remove 后清空选择并刷新列表', async () => {
    invokeMock.mockResolvedValue([srv()])
    const store = useServersStore()
    await store.load()
    invokeMock.mockResolvedValue([])
    await store.remove('s1')
    expect(invokeMock).toHaveBeenCalledWith('delete_server', { id: 's1' })
    expect(store.servers).toEqual([])
    expect(store.selectedId).toBeNull()
  })
})
```

- [ ] **Step 4: 跑测试确认失败**

Run: `pnpm vitest run src/stores`
Expected: 编译失败（store 不存在）

- [ ] **Step 5: 实现三个 store**

`src/stores/servers.ts`：
```ts
import { defineStore } from 'pinia'
import { api, onHostKeyPrompt, onTunnelEvent } from '../api'
import type { HostKeyPrompt, Server, ServerStatus, StatusEntry, UpsertServerInput } from '../types'

export const useServersStore = defineStore('servers', {
  state: () => ({
    servers: [] as Server[],
    selectedId: null as string | null,
    serverStatus: {} as Record<string, StatusEntry<ServerStatus>>,
    hostKeyPrompt: null as HostKeyPrompt | null,
  }),
  actions: {
    async load() {
      const [servers, snapshot] = await Promise.all([api.listServers(), api.getSnapshot()])
      this.servers = servers
      this.serverStatus = snapshot.servers
      if (!this.selectedId && servers.length > 0) this.selectedId = servers[0].id
      if (this.selectedId && !servers.some((s) => s.id === this.selectedId)) {
        this.selectedId = servers[0]?.id ?? null
      }
    },
    select(id: string) {
      this.selectedId = id
    },
    async save(input: UpsertServerInput) {
      await api.upsertServer(input)
      await this.load()
    },
    async remove(id: string) {
      await api.deleteServer(id)
      await this.load()
    },
    async connect(id: string) {
      await api.connectServer(id)
    },
    async disconnect(id: string) {
      await api.disconnectServer(id)
    },
    async respondHostKey(trust: boolean) {
      if (this.hostKeyPrompt) {
        await api.respondHostKey(this.hostKeyPrompt.prompt_id, trust)
        this.hostKeyPrompt = null
      }
    },
  },
})

// 事件绑定幂等:重复调用只绑一次
let bound = false
export function bindServersEvents() {
  if (bound) return
  bound = true
  onTunnelEvent((ev) => {
    if (ev.type === 'server_status') {
      const store = useServersStore()
      store.serverStatus[ev.server_id] = { status: ev.status, error: ev.error }
    }
  })
  onHostKeyPrompt((p) => {
    useServersStore().hostKeyPrompt = p
  })
}
```

`src/stores/forwards.ts`：
```ts
import { defineStore } from 'pinia'
import { api, onTunnelEvent } from '../api'
import type { Forward, ForwardStatus, StatusEntry } from '../types'

export const useForwardsStore = defineStore('forwards', {
  state: () => ({
    forwards: [] as Forward[],
    forwardStatus: {} as Record<string, StatusEntry<ForwardStatus>>,
  }),
  actions: {
    async load() {
      const [forwards, snapshot] = await Promise.all([api.listForwards(), api.getSnapshot()])
      this.forwards = forwards
      this.forwardStatus = snapshot.forwards
    },
    forwardsOf(serverId: string) {
      return this.forwards.filter((f) => f.server_id === serverId)
    },
    async save(forward: Forward) {
      await api.upsertForward(forward)
      await this.load()
    },
    async remove(id: string) {
      await api.deleteForward(id)
      await this.load()
    },
    async toggle(id: string) {
      const status = this.forwardStatus[id]?.status
      if (status === 'running' || status === 'starting') {
        await api.stopForward(id)
      } else {
        await api.startForward(id)
      }
    },
  },
})

let bound = false
export function bindForwardsEvents() {
  if (bound) return
  bound = true
  onTunnelEvent((ev) => {
    if (ev.type === 'forward_status') {
      const store = useForwardsStore()
      store.forwardStatus[ev.forward_id] = { status: ev.status, error: ev.error }
    }
  })
}
```

`src/stores/logs.ts`：
```ts
import { defineStore } from 'pinia'
import { api, onLog } from '../api'
import type { LogEntry } from '../types'

export const useLogsStore = defineStore('logs', {
  state: () => ({ entries: [] as LogEntry[] }),
  actions: {
    async load() {
      this.entries = await api.getLogs()
    },
    clear() {
      this.entries = []
    },
  },
})

let bound = false
export function bindLogsEvents() {
  if (bound) return
  bound = true
  onLog((entry) => {
    useLogsStore().entries.push(entry)
  })
}
```

- [ ] **Step 6: 跑测试确认通过**

Run: `pnpm vitest run src/stores`
Expected: 6 passed

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat: 前端 api 封装与 Pinia stores"
```

---

### Task 15: 前端视图组件 + 总验证

**Files:**
- Modify: `src/App.vue`、`src/main.ts`
- Create: `src/views/MainView.vue`、`src/views/SettingsView.vue`
- Create: `src/components/ServerEditorDialog.vue`、`src/components/ForwardEditorDialog.vue`、`src/components/HostKeyDialog.vue`、`src/components/LogPanel.vue`
- Test: `src/components/__tests__/ForwardEditorDialog.test.ts`

**Interfaces:**
- Consumes: Task 14 全部 store 与类型
- Produces: 完整可用的主窗口 UI

- [ ] **Step 1: main.ts 接入 Element Plus 与 Pinia**

```ts
import { createApp } from 'vue'
import { createPinia } from 'pinia'
import ElementPlus from 'element-plus'
import zhCn from 'element-plus/es/locale/lang/zh-cn'
import 'element-plus/dist/index.css'
import App from './App.vue'

createApp(App).use(createPinia()).use(ElementPlus, { locale: zhCn }).mount('#app')
```

- [ ] **Step 2: 写 ForwardEditorDialog 的失败测试**

`src/components/__tests__/ForwardEditorDialog.test.ts`：
```ts
import { mount } from '@vue/test-utils'
import ElementPlus from 'element-plus'
import ForwardEditorDialog from '../ForwardEditorDialog.vue'
import type { Forward } from '../../types'

function blank(): Forward {
  return {
    id: '', server_id: 's1', name: '', kind: 'local',
    bind_addr: '127.0.0.1', bind_port: 0, target_host: null, target_port: null,
    auto_start: false,
  }
}

describe('ForwardEditorDialog', () => {
  it('dynamic 类型隐藏目标字段', async () => {
    const wrapper = mount(ForwardEditorDialog, {
      props: { modelValue: true, forward: blank() },
      global: { plugins: [ElementPlus] },
    })
    expect(wrapper.text()).toContain('目标地址')
    wrapper.findComponent({ name: 'ElRadioGroup' }).vm.$emit('update:modelValue', 'dynamic')
    await wrapper.vm.$nextTick()
    expect(wrapper.text()).not.toContain('目标地址')
  })

  it('提交时校验:local/remote 必须填目标', async () => {
    const wrapper = mount(ForwardEditorDialog, {
      props: { modelValue: true, forward: blank() },
      global: { plugins: [ElementPlus] },
    })
    const emitted = wrapper.emitted('submit')
    await wrapper.find('form').trigger('submit.prevent')
    // 目标为空 → 不触发 submit
    expect(wrapper.emitted('submit')).toBeFalsy()
  })
})
```

- [ ] **Step 3: 跑测试确认失败**

Run: `pnpm vitest run src/components`
Expected: 失败（组件不存在）

- [ ] **Step 4: 实现 ForwardEditorDialog.vue**

```vue
<script setup lang="ts">
import { computed, reactive, watch } from 'vue'
import type { Forward, ForwardKind } from '../types'

const props = defineProps<{ modelValue: boolean; forward: Forward }>()
const emit = defineEmits<{ 'update:modelValue': [boolean]; submit: [Forward] }>()

// 本地副本,取消不落盘
const form = reactive<Forward>({ ...props.forward })
watch(() => props.forward, (f) => Object.assign(form, f))

const needTarget = computed(() => form.kind !== 'dynamic')
const bindLabel = computed(() => (form.kind === 'remote' ? '远程监听' : '本地监听'))

function submit() {
  if (!form.name || !form.bind_port) return
  if (needTarget.value && (!form.target_host || !form.target_port)) return
  if (form.kind === 'dynamic') {
    form.target_host = null
    form.target_port = null
  }
  emit('submit', { ...form })
  emit('update:modelValue', false)
}
</script>

<template>
  <el-dialog :model-value="modelValue" :title="form.id ? '编辑转发' : '添加转发'" width="480px"
    @update:model-value="emit('update:modelValue', $event)">
    <el-form label-width="90px" @submit.prevent="submit">
      <el-form-item label="名称" required>
        <el-input v-model="form.name" placeholder="例如:测试库 MySQL" />
      </el-form-item>
      <el-form-item label="类型">
        <el-radio-group v-model="form.kind">
          <el-radio-button value="local">本地 -L</el-radio-button>
          <el-radio-button value="remote">远程 -R</el-radio-button>
          <el-radio-button value="dynamic">SOCKS -D</el-radio-button>
        </el-radio-group>
      </el-form-item>
      <el-form-item :label="bindLabel" required>
        <el-input v-model="form.bind_addr" style="width: 60%" placeholder="127.0.0.1" />
        <el-input-number v-model="form.bind_port" :min="1" :max="65535" style="width: 38%; margin-left: 2%" />
      </el-form-item>
      <el-form-item v-if="needTarget" label="目标地址" required>
        <el-input v-model="form.target_host" style="width: 60%" placeholder="目标主机" />
        <el-input-number v-model="form.target_port" :min="1" :max="65535" style="width: 38%; margin-left: 2%" />
      </el-form-item>
      <el-form-item>
        <el-checkbox v-model="form.auto_start">应用启动时自动开启</el-checkbox>
      </el-form-item>
    </el-form>
    <template #footer>
      <el-button @click="emit('update:modelValue', false)">取消</el-button>
      <el-button type="primary" @click="submit">保存</el-button>
    </template>
  </el-dialog>
</template>
```

- [ ] **Step 5: 跑组件测试确认通过**

Run: `pnpm vitest run src/components`
Expected: 2 passed（若 el-radio-group 的 emit 测试写法与该版本 Element Plus 不符，改为直接修改 `wrapper.vm` 暴露的 form 不可行——可改成 mount 后找第二个 radio input 触发 click；以组件实际渲染为准调整测试）

- [ ] **Step 6: 实现其余组件**

`src/components/ServerEditorDialog.vue`（三个认证 tab，敏感值仅在用户填写时提交）：
```vue
<script setup lang="ts">
import { reactive, ref, watch } from 'vue'
import type { Server, UpsertServerInput } from '../types'

const props = defineProps<{ modelValue: boolean; server: Server | null }>()
const emit = defineEmits<{ 'update:modelValue': [boolean]; submit: [UpsertServerInput] }>()

const form = reactive({ name: '', host: '', port: 22, username: '' })
const authType = ref<'password' | 'key_file' | 'key_data'>('password')
const password = ref('')
const keyPath = ref('')
const keyData = ref('')
const keyPassphrase = ref('')

watch(
  () => props.server,
  (s) => {
    if (!s) {
      Object.assign(form, { name: '', host: '', port: 22, username: '' })
      authType.value = 'password'
      password.value = ''
      keyPath.value = ''
      keyData.value = ''
      keyPassphrase.value = ''
      return
    }
    Object.assign(form, { name: s.name, host: s.host, port: s.port, username: s.username })
    authType.value = s.auth.type
    if (s.auth.type === 'key_file') keyPath.value = s.auth.path
    // 敏感值不回填:留空表示保持不变
    password.value = ''
    keyData.value = ''
    keyPassphrase.value = ''
  },
  { immediate: true },
)

function submit() {
  if (!form.name || !form.host || !form.username) return
  const server: Server = {
    id: props.server?.id ?? '',
    name: form.name,
    host: form.host,
    port: form.port,
    username: form.username,
    auth:
      authType.value === 'password'
        ? { type: 'password' }
        : authType.value === 'key_file'
          ? { type: 'key_file', path: keyPath.value }
          : { type: 'key_data' },
  }
  emit('submit', {
    server,
    // 空字符串视为未修改,避免把钥匙串里的值清空
    password: password.value || null,
    key_data: keyData.value || null,
    key_passphrase: keyPassphrase.value || null,
  })
  emit('update:modelValue', false)
}
</script>

<template>
  <el-dialog :model-value="modelValue" :title="server ? '编辑服务器' : '添加服务器'" width="520px"
    @update:model-value="emit('update:modelValue', $event)">
    <el-form label-width="80px">
      <el-form-item label="名称" required><el-input v-model="form.name" /></el-form-item>
      <el-form-item label="主机" required>
        <el-input v-model="form.host" style="width: 65%" placeholder="域名或 IP" />
        <el-input-number v-model="form.port" :min="1" :max="65535" style="width: 33%; margin-left: 2%" />
      </el-form-item>
      <el-form-item label="用户名" required><el-input v-model="form.username" /></el-form-item>
      <el-form-item label="认证">
        <el-tabs v-model="authType" style="width: 100%">
          <el-tab-pane label="密码" name="password">
            <el-input v-model="password" type="password" show-password
              :placeholder="server ? '留空保持不变' : '登录密码'" />
          </el-tab-pane>
          <el-tab-pane label="密钥文件" name="key_file">
            <el-input v-model="keyPath" placeholder="如 ~/.ssh/id_ed25519" style="margin-bottom: 8px" />
            <el-input v-model="keyPassphrase" type="password" show-password placeholder="密钥密码(如有,留空保持不变)" />
          </el-tab-pane>
          <el-tab-pane label="粘贴密钥" name="key_data">
            <el-input v-model="keyData" type="textarea" :rows="6"
              :placeholder="server ? '留空保持不变' : '粘贴 -----BEGIN OPENSSH PRIVATE KEY----- 完整内容'" style="margin-bottom: 8px" />
            <el-input v-model="keyPassphrase" type="password" show-password placeholder="密钥密码(如有,留空保持不变)" />
          </el-tab-pane>
        </el-tabs>
      </el-form-item>
    </el-form>
    <template #footer>
      <el-button @click="emit('update:modelValue', false)">取消</el-button>
      <el-button type="primary" @click="submit">保存</el-button>
    </template>
  </el-dialog>
</template>
```

`src/components/HostKeyDialog.vue`：
```vue
<script setup lang="ts">
import { computed } from 'vue'
import { useServersStore } from '../stores/servers'

const store = useServersStore()
const prompt = computed(() => store.hostKeyPrompt)
</script>

<template>
  <el-dialog :model-value="!!prompt" :title="prompt?.is_mismatch ? '警告:主机密钥已变更' : '信任新的主机密钥?'"
    width="480px" :close-on-click-modal="false" :show-close="false">
    <template v-if="prompt">
      <p><b>{{ prompt.host }}:{{ prompt.port }}</b></p>
      <p>指纹:<code>{{ prompt.fingerprint }}</code></p>
      <el-alert v-if="prompt.is_mismatch" type="error" :closable="false"
        title="与已记录的密钥不符,连接可能被劫持。确认服务器重装/换 key 后再信任。" />
      <el-alert v-else type="warning" :closable="false" title="首次连接该主机,信任后将记录此密钥。" />
    </template>
    <template #footer>
      <el-button @click="store.respondHostKey(false)">拒绝</el-button>
      <el-button :type="prompt?.is_mismatch ? 'danger' : 'primary'" @click="store.respondHostKey(true)">信任并继续</el-button>
    </template>
  </el-dialog>
</template>
```

`src/components/LogPanel.vue`：
```vue
<script setup lang="ts">
import { onMounted } from 'vue'
import { useLogsStore } from '../stores/logs'

const store = useLogsStore()
onMounted(() => store.load())
</script>

<template>
  <div class="log-panel">
    <div class="log-toolbar">
      <el-button size="small" @click="store.load()">刷新</el-button>
      <el-button size="small" @click="store.clear()">清空显示</el-button>
    </div>
    <div class="log-list">
      <div v-for="(e, i) in store.entries" :key="i" :class="['log-line', e.level.toLowerCase()]">
        <span class="ts">{{ e.timestamp }}</span> <span class="lv">{{ e.level }}</span> {{ e.message }}
      </div>
      <el-empty v-if="store.entries.length === 0" description="暂无日志" :image-size="60" />
    </div>
  </div>
</template>

<style scoped>
.log-panel { display: flex; flex-direction: column; height: 100%; }
.log-list { flex: 1; overflow: auto; font-family: monospace; font-size: 12px; }
.log-line .ts { color: #999; margin-right: 6px; }
.log-line.error .lv { color: #f56c6c; }
.log-line.warn .lv { color: #e6a23c; }
</style>
```

`src/views/MainView.vue`（左服务器列表 + 右隧道表格）：
```vue
<script setup lang="ts">
import { computed, ref } from 'vue'
import { useServersStore } from '../stores/servers'
import { useForwardsStore } from '../stores/forwards'
import ServerEditorDialog from '../components/ServerEditorDialog.vue'
import ForwardEditorDialog from '../components/ForwardEditorDialog.vue'
import type { Forward, Server, UpsertServerInput } from '../types'

const servers = useServersStore()
const forwards = useForwardsStore()

const serverDialog = ref(false)
const editingServer = ref<Server | null>(null)
const forwardDialog = ref(false)
const editingForward = ref<Forward | null>(null)

const currentForwards = computed(() => (servers.selectedId ? forwards.forwardsOf(servers.selectedId) : []))

function blankForward(): Forward {
  return {
    id: '', server_id: servers.selectedId ?? '', name: '', kind: 'local',
    bind_addr: '127.0.0.1', bind_port: 0, target_host: null, target_port: null, auto_start: false,
  }
}

function statusText(s?: string) {
  return { connected: '已连接', connecting: '连接中', reconnecting: '重连中', error: '错误' }[s ?? ''] ?? '未连接'
}
function forwardStatusText(s?: string) {
  return { running: '运行中', starting: '启动中', error: '错误' }[s ?? ''] ?? '已停止'
}

async function saveServer(input: UpsertServerInput) {
  await servers.save(input)
}
async function saveForward(f: Forward) {
  await forwards.save(f)
}

defineExpose({
  openAddForward(serverId?: string) {
    if (serverId) servers.select(serverId)
    editingForward.value = blankForward()
    forwardDialog.value = true
  },
})
</script>

<template>
  <div class="main-view">
    <aside class="server-list">
      <div class="list-header">
        <span>服务器</span>
        <el-button size="small" type="primary" @click="editingServer = null; serverDialog = true">添加</el-button>
      </div>
      <div v-for="s in servers.servers" :key="s.id"
        :class="['server-item', { active: s.id === servers.selectedId }]" @click="servers.select(s.id)">
        <div class="server-name">{{ s.name }}</div>
        <div class="server-sub">{{ s.username }}@{{ s.host }}:{{ s.port }}</div>
        <div class="server-status" :class="servers.serverStatus[s.id]?.status">
          {{ statusText(servers.serverStatus[s.id]?.status) }}
        </div>
        <div class="server-actions">
          <el-button size="small" text @click.stop="editingServer = s; serverDialog = true">编辑</el-button>
          <el-popconfirm title="删除该服务器及其全部转发?" @confirm="servers.remove(s.id)">
            <template #reference><el-button size="small" text type="danger" @click.stop>删除</el-button></template>
          </el-popconfirm>
        </div>
      </div>
      <el-empty v-if="servers.servers.length === 0" description="还没有服务器,点击「添加」" :image-size="80" />
    </aside>

    <section class="forward-panel">
      <div class="list-header">
        <span>端口转发</span>
        <el-button size="small" type="primary" :disabled="!servers.selectedId"
          @click="editingForward = blankForward(); forwardDialog = true">添加转发</el-button>
      </div>
      <el-table :data="currentForwards" style="width: 100%">
        <el-table-column prop="name" label="名称" min-width="110" />
        <el-table-column label="类型" width="90">
          <template #default="{ row }">{{ { local: '本地', remote: '远程', dynamic: 'SOCKS' }[row.kind as string] }}</template>
        </el-table-column>
        <el-table-column label="监听" width="130">
          <template #default="{ row }">{{ row.bind_addr }}:{{ row.bind_port }}</template>
        </el-table-column>
        <el-table-column label="目标" min-width="140">
          <template #default="{ row }">
            <span v-if="row.kind !== 'dynamic'">{{ row.target_host }}:{{ row.target_port }}</span>
            <span v-else>—</span>
          </template>
        </el-table-column>
        <el-table-column label="状态" width="160">
          <template #default="{ row }">
            <el-tooltip :content="forwards.forwardStatus[row.id]?.error ?? ''"
              :disabled="!forwards.forwardStatus[row.id]?.error">
              <span :class="['fwd-status', forwards.forwardStatus[row.id]?.status]">
                {{ forwardStatusText(forwards.forwardStatus[row.id]?.status) }}
              </span>
            </el-tooltip>
          </template>
        </el-table-column>
        <el-table-column label="操作" width="190">
          <template #default="{ row }">
            <el-switch
              :model-value="['running', 'starting'].includes(forwards.forwardStatus[row.id]?.status ?? '')"
              @change="forwards.toggle(row.id)" />
            <el-button size="small" text @click="editingForward = { ...row }; forwardDialog = true">编辑</el-button>
            <el-popconfirm title="删除该转发?" @confirm="forwards.remove(row.id)">
              <template #reference><el-button size="small" text type="danger">删除</el-button></template>
            </el-popconfirm>
          </template>
        </el-table-column>
      </el-table>
    </section>

    <ServerEditorDialog v-model="serverDialog" :server="editingServer" @submit="saveServer" />
    <ForwardEditorDialog v-if="editingForward" v-model="forwardDialog" :forward="editingForward" @submit="saveForward" />
  </div>
</template>

<style scoped>
.main-view { display: flex; height: 100%; }
.server-list { width: 240px; border-right: 1px solid #e4e7ed; overflow: auto; padding: 8px; }
.list-header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 8px; font-weight: 600; }
.server-item { padding: 8px; border-radius: 6px; cursor: pointer; margin-bottom: 4px; }
.server-item.active { background: #ecf5ff; }
.server-name { font-weight: 600; }
.server-sub { font-size: 12px; color: #909399; }
.server-status { font-size: 12px; color: #909399; }
.server-status.connected { color: #67c23a; }
.server-status.error, .server-status.reconnecting { color: #f56c6c; }
.forward-panel { flex: 1; padding: 8px 16px; overflow: auto; }
.fwd-status.running { color: #67c23a; }
.fwd-status.error { color: #f56c6c; }
</style>
```

`src/views/SettingsView.vue`：
```vue
<script setup lang="ts">
import { onMounted, reactive } from 'vue'
import { ElMessage } from 'element-plus'
import { api } from '../api'
import type { Settings } from '../types'

const form = reactive<Settings>({ auto_reconnect: true, minimize_to_tray: true, launch_at_login: false })

onMounted(async () => {
  Object.assign(form, await api.getSettings())
})

async function save() {
  await api.saveSettings({ ...form })
  ElMessage.success('已保存')
}
</script>

<template>
  <div class="settings-view">
    <el-form label-width="180px" style="max-width: 480px">
      <el-form-item label="断线自动重连">
        <el-switch v-model="form.auto_reconnect" />
      </el-form-item>
      <el-form-item label="关闭窗口时最小化到托盘">
        <el-switch v-model="form.minimize_to_tray" />
      </el-form-item>
      <el-form-item label="开机自启动">
        <el-switch v-model="form.launch_at_login" />
      </el-form-item>
      <el-form-item><el-button type="primary" @click="save">保存</el-button></el-form-item>
    </el-form>
  </div>
</template>
```

`src/App.vue`（tab 框架 + 事件绑定 + host key 弹窗 + 托盘导航）：
```vue
<script setup lang="ts">
import { onMounted, ref } from 'vue'
import MainView from './views/MainView.vue'
import SettingsView from './views/SettingsView.vue'
import LogPanel from './components/LogPanel.vue'
import HostKeyDialog from './components/HostKeyDialog.vue'
import { onNavigate } from './api'
import { useServersStore } from './stores/servers'
import { useForwardsStore, bindForwardsEvents } from './stores/forwards'
import { bindServersEvents } from './stores/servers'
import { bindLogsEvents } from './stores/logs'

const tab = ref('main')
const mainView = ref<InstanceType<typeof MainView>>()

onMounted(async () => {
  const servers = useServersStore()
  const forwards = useForwardsStore()
  bindServersEvents()
  bindForwardsEvents()
  bindLogsEvents()
  await Promise.all([servers.load(), forwards.load()])
  onNavigate((nav) => {
    tab.value = 'main'
    if (nav.view === 'add-forward') mainView.value?.openAddForward(nav.server_id)
  })
})
</script>

<template>
  <el-container class="app-shell">
    <el-header class="app-header" height="48px">
      <span class="title">SSH Tunnel</span>
      <el-radio-group v-model="tab" size="small">
        <el-radio-button value="main">转发</el-radio-button>
        <el-radio-button value="logs">日志</el-radio-button>
        <el-radio-button value="settings">设置</el-radio-button>
      </el-radio-group>
    </el-header>
    <el-main class="app-main">
      <MainView v-show="tab === 'main'" ref="mainView" />
      <LogPanel v-show="tab === 'logs'" />
      <SettingsView v-show="tab === 'settings'" />
    </el-main>
    <HostKeyDialog />
  </el-container>
</template>

<style>
html, body, #app { height: 100%; margin: 0; }
.app-shell { height: 100%; }
.app-header { display: flex; align-items: center; gap: 16px; border-bottom: 1px solid #e4e7ed; }
.app-header .title { font-weight: 700; }
.app-main { padding: 0; }
</style>
```

- [ ] **Step 7: 总验证**

Run:
```bash
pnpm test && pnpm type-check && pnpm build
cargo test -p ssh-tunnel-core
cargo build -p ssh-tunnel-app
```
Expected: 全绿。

手动冒烟（有桌面环境时；当前无头环境跳过并告知用户）：
```bash
pnpm tauri dev
```
清单：添加服务器（密码）→ 添加本地转发 → 开关启停 → 关闭窗口进托盘 → 托盘右键启停 → 托盘添加转发跳转 → host key 弹窗 → 退出。

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "feat: 前端界面（主视图、编辑对话框、日志、设置）"
```

---

## 后续（不在本计划）

- 打包图标（`pnpm tauri icon`）与 GitHub Actions 三平台出包
- 跳板机 / ProxyJump、SFTP、终端
- Windows/macOS 手动验证
