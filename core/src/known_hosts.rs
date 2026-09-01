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

/// 去掉 comment 的归一化副本。
/// russh 的 check_known_hosts_path 用 `PublicKey ==` 比较，而 ssh-key 的
/// PartialEq 包含 comment 字段；known_hosts 里解析出来的 key comment 为空，
/// 带 comment 的 key 会被误判成 Changed，所以比较与落盘前统一清空。
fn normalize(key: &PublicKey) -> PublicKey {
    let mut k = key.clone();
    k.set_comment("");
    k
}

impl KnownHosts {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn check(
        &self,
        host: &str,
        port: u16,
        key: &PublicKey,
    ) -> Result<HostKeyStatus, CoreError> {
        if !self.path.exists() {
            return Ok(HostKeyStatus::Unknown);
        }
        let key = normalize(key);
        match russh::keys::check_known_hosts_path(host, port, &key, &self.path) {
            Ok(true) => Ok(HostKeyStatus::Trusted),
            Ok(false) => Ok(HostKeyStatus::Unknown),
            Err(russh::keys::Error::KeyChanged { .. }) => Ok(HostKeyStatus::Changed),
            Err(e) => Err(e.into()),
        }
    }

    pub fn record(&self, host: &str, port: u16, key: &PublicKey) -> Result<(), CoreError> {
        // learn_known_hosts_path 未在 russh::keys 顶层 re-export，需走子模块路径
        russh::keys::known_hosts::learn_known_hosts_path(host, port, &normalize(key), &self.path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 一次性生成的测试密钥（仅测试用，对应公钥由此私钥现算）
    const TEST_KEY_PEM: &str = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW\nQyNTUxOQAAACA+DoeanTBWDxrSxMpB7n99cAYBH+KJWA6k1w3F5kjhVAAAAJg/lSznP5Us\n5wAAAAtzc2gtZWQyNTUxOQAAACA+DoeanTBWDxrSxMpB7n99cAYBH+KJWA6k1w3F5kjhVA\nAAAEBwkxfG4Qcvs76Hgt3QhMo3pA3dpAVPHmmq3IvfR5hhxD4Oh5qdMFYPGtLEykHuf31w\nBgEf4olYDqTXDcXmSOFUAAAAD3NzaC10dW5uZWwtdGVzdAECAwQFBg==\n-----END OPENSSH PRIVATE KEY-----\n";

    fn test_pubkey() -> russh::keys::PublicKey {
        russh::keys::decode_secret_key(TEST_KEY_PEM, None)
            .unwrap()
            .public_key()
            .clone()
    }

    // 另一把不同的密钥（同样为静态 ed25519 测试密钥），用于模拟 host key 变更。
    // 注意必须是同算法：算法不同会被 check_known_hosts_path 判为「无匹配」而非「变更」。
    const OTHER_KEY_PEM: &str = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW\nQyNTUxOQAAACCZ4teB08q/8mdT5HlwI+cTU8YfHDMNrz7zrx/3phLEBQAAAJjYcr1+2HK9\nfgAAAAtzc2gtZWQyNTUxOQAAACCZ4teB08q/8mdT5HlwI+cTU8YfHDMNrz7zrx/3phLEBQ\nAAAEDx44VbI8jaRl0XTBZ3I1xGYt78MaPGmXOxoxu5fFxda5ni14HTyr/yZ1PkeXAj5xNT\nxh8cMw2vPvOvH/emEsQFAAAAEXNzaC10dW5uZWwtdGVzdC0yAQIDBA==\n-----END OPENSSH PRIVATE KEY-----\n";

    fn other_pubkey() -> russh::keys::PublicKey {
        russh::keys::decode_secret_key(OTHER_KEY_PEM, None)
            .unwrap()
            .public_key()
            .clone()
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
        assert_eq!(
            kh.check("h1", 22, &other_pubkey()).unwrap(),
            HostKeyStatus::Changed
        );
    }

    #[test]
    fn different_host_port_independent() {
        let dir = tempfile::tempdir().unwrap();
        let kh = KnownHosts::new(dir.path().join("known_hosts"));
        kh.record("h1", 22, &test_pubkey()).unwrap();
        assert_eq!(
            kh.check("h1", 2222, &test_pubkey()).unwrap(),
            HostKeyStatus::Unknown
        );
        assert_eq!(
            kh.check("h2", 22, &test_pubkey()).unwrap(),
            HostKeyStatus::Unknown
        );
    }
}
