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
    let handle = conn.handle;
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
