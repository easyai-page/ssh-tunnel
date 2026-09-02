use crate::CoreError;
use std::collections::HashMap;
use std::path::PathBuf;
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

/// 同步 trait：底层是阻塞 IO（keyring/文件），调用方负责 spawn_blocking
pub trait SecretStore: Send + Sync {
    fn get(&self, server_id: &str, kind: SecretKind) -> Result<Option<String>, CoreError>;
    fn set(&self, server_id: &str, kind: SecretKind, value: &str) -> Result<(), CoreError>;
    fn delete(&self, server_id: &str, kind: SecretKind) -> Result<(), CoreError>;
}

const SERVICE: &str = "ssh-tunnel";

#[derive(Default)]
pub struct KeyringStore;

impl KeyringStore {
    pub fn new() -> Self {
        Self
    }
    fn entry(server_id: &str, kind: SecretKind) -> Result<keyring::Entry, CoreError> {
        Ok(keyring::Entry::new(
            SERVICE,
            &secret_account(server_id, kind),
        )?)
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

/// 生产环境存储：密码/密钥 passphrase（短，几十字节）走系统钥匙串；
/// 粘贴的私钥内容落盘到 <配置目录>/keys/<server_id>。
/// 原因：Windows 凭据管理器单条 blob 上限 2560 字节（UTF-16 存，约 1280 字符），
/// RSA 私钥轻松超限、写钥匙串直接报错；文件存储无此限制
pub struct HybridSecretStore {
    keyring: KeyringStore,
    keys_dir: PathBuf,
}

impl HybridSecretStore {
    pub fn new(keys_dir: PathBuf) -> Self {
        Self {
            keyring: KeyringStore::new(),
            keys_dir,
        }
    }

    fn key_path(&self, server_id: &str) -> PathBuf {
        self.keys_dir.join(server_id)
    }

    fn file_get(&self, server_id: &str) -> Result<Option<String>, CoreError> {
        match std::fs::read_to_string(self.key_path(server_id)) {
            Ok(v) => Ok(Some(v)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn file_set(&self, server_id: &str, value: &str) -> Result<(), CoreError> {
        std::fs::create_dir_all(&self.keys_dir)?;
        let path = self.key_path(server_id);
        // 与 ConfigStore 同规约：先写临时文件再 rename，避免半截文件
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, value)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        }
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    fn file_delete(&self, server_id: &str) -> Result<(), CoreError> {
        match std::fs::remove_file(self.key_path(server_id)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

impl SecretStore for HybridSecretStore {
    fn get(&self, server_id: &str, kind: SecretKind) -> Result<Option<String>, CoreError> {
        match kind {
            SecretKind::Key => self.file_get(server_id),
            _ => self.keyring.get(server_id, kind),
        }
    }
    fn set(&self, server_id: &str, kind: SecretKind, value: &str) -> Result<(), CoreError> {
        match kind {
            SecretKind::Key => self.file_set(server_id, value),
            _ => self.keyring.set(server_id, kind, value),
        }
    }
    fn delete(&self, server_id: &str, kind: SecretKind) -> Result<(), CoreError> {
        match kind {
            SecretKind::Key => self.file_delete(server_id),
            _ => self.keyring.delete(server_id, kind),
        }
    }
}

/// 测试与无桌面环境下的内存实现
#[derive(Default)]
pub struct MemorySecretStore {
    inner: Mutex<HashMap<(String, SecretKind), String>>,
}

impl MemorySecretStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl SecretStore for MemorySecretStore {
    fn get(&self, server_id: &str, kind: SecretKind) -> Result<Option<String>, CoreError> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .get(&(server_id.to_string(), kind))
            .cloned())
    }
    fn set(&self, server_id: &str, kind: SecretKind, value: &str) -> Result<(), CoreError> {
        self.inner
            .lock()
            .unwrap()
            .insert((server_id.to_string(), kind), value.to_string());
        Ok(())
    }
    fn delete(&self, server_id: &str, kind: SecretKind) -> Result<(), CoreError> {
        self.inner
            .lock()
            .unwrap()
            .remove(&(server_id.to_string(), kind));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_naming() {
        assert_eq!(secret_account("s1", SecretKind::Password), "s1:password");
        assert_eq!(secret_account("s1", SecretKind::Key), "s1:key");
        assert_eq!(
            secret_account("s1", SecretKind::KeyPassphrase),
            "s1:key_passphrase"
        );
    }

    #[test]
    fn hybrid_store_key_kind_uses_file() {
        // 只测 SecretKind::Key(走文件);其余 kind 会碰真实 keyring,不进单元测试
        let dir = tempfile::tempdir().unwrap();
        let store = HybridSecretStore::new(dir.path().join("keys"));
        assert_eq!(store.get("s1", SecretKind::Key).unwrap(), None);
        let pem = "-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n-----END OPENSSH PRIVATE KEY-----\n";
        store.set("s1", SecretKind::Key, pem).unwrap();
        assert_eq!(
            store.get("s1", SecretKind::Key).unwrap().as_deref(),
            Some(pem)
        );
        // 覆盖写与删除
        store.set("s1", SecretKind::Key, "k2").unwrap();
        assert_eq!(
            store.get("s1", SecretKind::Key).unwrap().as_deref(),
            Some("k2")
        );
        store.delete("s1", SecretKind::Key).unwrap();
        assert_eq!(store.get("s1", SecretKind::Key).unwrap(), None);
        // 删除不存在的条目不算错误
        store.delete("s1", SecretKind::Key).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn hybrid_store_key_file_is_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let store = HybridSecretStore::new(dir.path().join("keys"));
        store.set("s1", SecretKind::Key, "k").unwrap();
        let mode = std::fs::metadata(dir.path().join("keys/s1"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn memory_store_roundtrip() {
        let store = MemorySecretStore::new();
        assert_eq!(store.get("s1", SecretKind::Password).unwrap(), None);
        store.set("s1", SecretKind::Password, "pw").unwrap();
        assert_eq!(
            store.get("s1", SecretKind::Password).unwrap().as_deref(),
            Some("pw")
        );
        store.set("s1", SecretKind::Password, "pw2").unwrap();
        assert_eq!(
            store.get("s1", SecretKind::Password).unwrap().as_deref(),
            Some("pw2")
        );
        store.delete("s1", SecretKind::Password).unwrap();
        assert_eq!(store.get("s1", SecretKind::Password).unwrap(), None);
        // 不同 kind / 不同 server 互不影响
        store.set("s1", SecretKind::Key, "k").unwrap();
        store.set("s2", SecretKind::Key, "k2").unwrap();
        assert_eq!(
            store.get("s1", SecretKind::Key).unwrap().as_deref(),
            Some("k")
        );
    }
}
