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

#[tokio::test]
async fn start_forward_during_reconnecting_connects_once() {
    let echo = start_tcp_echo().await;
    let ssh = start_ssh_server(TestServerOpts { password: Some(TEST_PASSWORD), accept_keys: vec![] }).await;
    let addr = ssh.addr;
    let secrets = Arc::new(MemorySecretStore::new());
    secrets.set("s1", SecretKind::Password, TEST_PASSWORD).unwrap();
    let (events, mut rx) = broadcast::channel(64);
    let kh = Arc::new(Mutex::new(KnownHosts::new(tempfile::tempdir().unwrap().path().join("kh"))));
    let actor = spawn_actor(test_server(addr), secrets, kh, decider(), true, events);

    actor.send(ActorCommand::Connect).unwrap();
    wait_server_status(&mut rx, ServerStatus::Connected).await;

    // 杀掉服务器 → Reconnecting,重试排程在 1s 后
    ssh.shutdown.shutdown("boom".into());
    wait_server_status(&mut rx, ServerStatus::Reconnecting).await;

    // 抢在重试定时器触发前:同端口重启服务器,发 StartForward 走隐式连接
    let ssh2 = start_ssh_server_on(addr, TestServerOpts { password: Some(TEST_PASSWORD), accept_keys: vec![] }).await;
    let listener_port = {
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
    actor.send(ActorCommand::StartForward(fwd)).unwrap();
    wait_server_status(&mut rx, ServerStatus::Connected).await;
    wait_forward_status(&mut rx, ForwardStatus::Running).await;

    // 隐式连接成功必须取消已排程的重试:2.5s 窗口(覆盖原 1s 重试点)内
    // 不得再出现 Connecting/Connected,否则说明旧定时器二次连接、替换了健康连接
    let mut second_connect = false;
    let deadline = std::time::Instant::now() + Duration::from_millis(2500);
    while let Ok(Ok(ev)) = tokio::time::timeout(deadline - std::time::Instant::now(), rx.recv()).await.map_err(|_| ()).map(|r| r) {
        if let TunnelEvent::ServerStatus { status: ServerStatus::Connecting | ServerStatus::Connected, .. } = ev {
            second_connect = true;
        }
    }
    assert!(!second_connect, "Reconnecting 期间 StartForward 隐式连上后出现了二次连接事件");

    // 唯一的那条连接上,转发功能正常
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut client = tokio::net::TcpStream::connect(("127.0.0.1", listener_port)).await.unwrap();
    client.write_all(b"once").await.unwrap();
    let mut buf = vec![0u8; 4];
    client.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"once");
    actor.send(ActorCommand::Shutdown).unwrap();
    ssh2.shutdown.shutdown("done".into());
}
