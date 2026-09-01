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
