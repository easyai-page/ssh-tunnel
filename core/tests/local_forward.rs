mod support;

use ssh_tunnel_core::forward::local::{bind_listener, spawn_local_forward};
use ssh_tunnel_core::known_hosts::KnownHosts;
use ssh_tunnel_core::model::{AuthMethod, Server};
use ssh_tunnel_core::secrets::{MemorySecretStore, SecretKind, SecretStore};
use ssh_tunnel_core::ssh::client::{connect, ChannelOpener, OpenChannelRequest};
use ssh_tunnel_core::CoreError;
use std::sync::Arc;
use support::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, Mutex};

async fn connected_opener(
    ssh_addr: std::net::SocketAddr,
) -> (ChannelOpener, tokio::task::JoinHandle<()>) {
    let secrets = Arc::new(MemorySecretStore::new());
    secrets
        .set("s1", SecretKind::Password, TEST_PASSWORD)
        .unwrap();
    let server = Server {
        id: "s1".into(),
        name: "t".into(),
        host: ssh_addr.ip().to_string(),
        port: ssh_addr.port(),
        username: "u".into(),
        auth: AuthMethod::Password,
    };
    let dir = tempfile::tempdir().unwrap();
    let kh = Arc::new(Mutex::new(KnownHosts::new(
        Box::leak(Box::new(dir)).path().join("kh"),
    )));
    let decider: ssh_tunnel_core::ssh::client::HostKeyDecider =
        Arc::new(|_| Box::pin(async { true }));
    let conn = connect(&server, secrets, kh, decider).await.unwrap();
    // 模拟 actor:持有 handle,响应开通道请求
    let (tx, mut rx) = mpsc::channel::<OpenChannelRequest>(32);
    let handle = conn.handle;
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
    let ssh = start_ssh_server(TestServerOpts {
        password: Some(TEST_PASSWORD),
        accept_keys: vec![],
    })
    .await;
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
