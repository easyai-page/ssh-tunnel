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
        Self {
            version: 1,
            servers: vec![],
            forwards: vec![],
            settings: Settings::default(),
        }
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
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        }
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    fn sample_config() -> AppConfig {
        AppConfig {
            version: 1,
            servers: vec![Server {
                id: "s1".into(),
                name: "db".into(),
                host: "10.0.0.2".into(),
                port: 22,
                username: "u".into(),
                auth: AuthMethod::Password,
            }],
            forwards: vec![Forward {
                id: "f1".into(),
                server_id: "s1".into(),
                name: "mysql".into(),
                kind: ForwardKind::Local,
                bind_addr: "127.0.0.1".into(),
                bind_port: 3306,
                target_host: Some("127.0.0.1".into()),
                target_port: Some(3306),
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
        let mode = std::fs::metadata(dir.path().join("config.json"))
            .unwrap()
            .permissions()
            .mode();
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
