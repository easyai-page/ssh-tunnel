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
