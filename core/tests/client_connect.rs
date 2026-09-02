mod support;

use ssh_tunnel_core::known_hosts::KnownHosts;
use ssh_tunnel_core::model::{AuthMethod, Server};
use ssh_tunnel_core::secrets::{MemorySecretStore, SecretKind, SecretStore};
use ssh_tunnel_core::ssh::client::{connect, HostKeyDecider};
use ssh_tunnel_core::CoreError;
use std::sync::Arc;
use support::*;
use tokio::sync::Mutex;

fn test_server(addr: std::net::SocketAddr, auth: AuthMethod) -> Server {
    Server {
        id: "s1".into(),
        name: "t".into(),
        host: addr.ip().to_string(),
        port: addr.port(),
        username: "u".into(),
        auth,
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
    let server = start_ssh_server(TestServerOpts {
        password: Some(TEST_PASSWORD),
        accept_keys: vec![],
    })
    .await;
    let secrets = Arc::new(MemorySecretStore::new());
    secrets
        .set("s1", SecretKind::Password, TEST_PASSWORD)
        .unwrap();
    let conn = connect(
        &test_server(server.addr, AuthMethod::Password),
        secrets,
        temp_known_hosts(),
        always_trust(),
    )
    .await;
    assert!(conn.is_ok(), "连接应成功: {:?}", conn.err());
}

#[tokio::test]
async fn password_auth_failure() {
    let server = start_ssh_server(TestServerOpts {
        password: Some(TEST_PASSWORD),
        accept_keys: vec![],
    })
    .await;
    let secrets = Arc::new(MemorySecretStore::new());
    secrets.set("s1", SecretKind::Password, "wrong").unwrap();
    let result = connect(
        &test_server(server.addr, AuthMethod::Password),
        secrets,
        temp_known_hosts(),
        always_trust(),
    )
    .await;
    assert!(matches!(result, Err(CoreError::Auth(_))));
}

#[tokio::test]
async fn missing_password_is_auth_error() {
    let server = start_ssh_server(TestServerOpts {
        password: Some(TEST_PASSWORD),
        accept_keys: vec![],
    })
    .await;
    let secrets = Arc::new(MemorySecretStore::new());
    let result = connect(
        &test_server(server.addr, AuthMethod::Password),
        secrets,
        temp_known_hosts(),
        always_trust(),
    )
    .await;
    assert!(matches!(result, Err(CoreError::Auth(_))));
}

#[tokio::test]
async fn key_data_auth_success() {
    // 授权公钥由测试客户端私钥现算。
    // 注意清空 comment：to_openssh 会带上私钥里的 comment（"ssh-tunnel-test"），
    // 而 SSH 协议线上不传 comment，服务器侧收到的 key comment 为空，
    // 测试服务器按字符串比较 authorized_keys，不归一就会误判拒绝
    let mut pk = russh::keys::decode_secret_key(TEST_CLIENT_KEY, None)
        .unwrap()
        .public_key()
        .clone();
    pk.set_comment("");
    let pubkey = pk.to_openssh().unwrap();
    let server = start_ssh_server(TestServerOpts {
        password: None,
        accept_keys: vec![pubkey],
    })
    .await;
    let secrets = Arc::new(MemorySecretStore::new());
    secrets.set("s1", SecretKind::Key, TEST_CLIENT_KEY).unwrap();
    let conn = connect(
        &test_server(server.addr, AuthMethod::KeyData),
        secrets,
        temp_known_hosts(),
        always_trust(),
    )
    .await;
    assert!(conn.is_ok(), "密钥认证应成功: {:?}", conn.err());
}

#[tokio::test]
async fn untrusted_host_key_aborts() {
    let server = start_ssh_server(TestServerOpts {
        password: Some(TEST_PASSWORD),
        accept_keys: vec![],
    })
    .await;
    let secrets = Arc::new(MemorySecretStore::new());
    secrets
        .set("s1", SecretKind::Password, TEST_PASSWORD)
        .unwrap();
    let result = connect(
        &test_server(server.addr, AuthMethod::Password),
        secrets,
        temp_known_hosts(),
        always_reject(),
    )
    .await;
    assert!(matches!(result, Err(CoreError::HostKeyRejected)));
}

#[tokio::test]
async fn trusted_key_persisted_and_reused() {
    let server = start_ssh_server(TestServerOpts {
        password: Some(TEST_PASSWORD),
        accept_keys: vec![],
    })
    .await;
    let secrets = Arc::new(MemorySecretStore::new());
    secrets
        .set("s1", SecretKind::Password, TEST_PASSWORD)
        .unwrap();
    let kh = temp_known_hosts();
    let s = test_server(server.addr, AuthMethod::Password);
    // 第一次:decider 信任并记录;第二次:decider 拒绝也应成功(已信任)
    connect(&s, secrets.clone(), kh.clone(), always_trust())
        .await
        .unwrap();
    let conn2 = connect(&s, secrets, kh, always_reject()).await;
    assert!(
        conn2.is_ok(),
        "已记录的 host key 不应再询问: {:?}",
        conn2.err()
    );
}

#[tokio::test]
async fn disconnect_notified_when_server_gone() {
    let server = start_ssh_server(TestServerOpts {
        password: Some(TEST_PASSWORD),
        accept_keys: vec![],
    })
    .await;
    let secrets = Arc::new(MemorySecretStore::new());
    secrets
        .set("s1", SecretKind::Password, TEST_PASSWORD)
        .unwrap();
    let mut conn = connect(
        &test_server(server.addr, AuthMethod::Password),
        secrets,
        temp_known_hosts(),
        always_trust(),
    )
    .await
    .unwrap();
    server.shutdown.shutdown("bye".into());
    let msg =
        tokio::time::timeout(std::time::Duration::from_secs(5), conn.disconnect_rx.recv()).await;
    assert!(msg.is_ok(), "服务器关停后应收到断线通知");
}
