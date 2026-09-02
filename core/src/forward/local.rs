use crate::ssh::client::ChannelOpener;
use crate::CoreError;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

pub async fn bind_listener(addr: &str, port: u16) -> Result<TcpListener, CoreError> {
    TcpListener::bind((addr, port)).await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::AddrInUse {
            CoreError::PortInUse(port)
        } else {
            CoreError::Io(e)
        }
    })
}

/// 本地转发 accept 循环。listener 跨重连存活:每次 accept 时才向 actor 请求开通道,
/// 因此连接断开重建后无需重新绑定端口(避免重连时端口竞争)
pub fn spawn_local_forward(
    listener: TcpListener,
    opener: ChannelOpener,
    target_host: String,
    target_port: u16,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let opener = opener.clone();
            let (host, port) = (target_host.clone(), target_port as u32);
            tokio::spawn(async move {
                match opener.open(&host, port).await {
                    Ok(channel) => {
                        let mut stream = channel.into_stream();
                        let _ = tokio::io::copy_bidirectional(&mut stream, &mut socket).await;
                    }
                    Err(e) => tracing::warn!("开 direct-tcpip 通道失败: {e}"),
                }
            });
        }
    })
}
