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

fn local_echo_target(echo: std::net::SocketAddr, _ssh_addr: std::net::SocketAddr) -> Forward {
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
