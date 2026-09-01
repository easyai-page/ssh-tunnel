use crate::socks5::{socks5_accept_target, socks5_reply};
use crate::ssh::client::ChannelOpener;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

pub fn spawn_socks_forward(listener: TcpListener, opener: ChannelOpener) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else { break };
            let opener = opener.clone();
            tokio::spawn(async move {
                let result = async {
                    let (host, port) = socks5_accept_target(&mut socket).await?;
                    match opener.open(&host, port as u32).await {
                        Ok(channel) => {
                            socks5_reply(&mut socket, true).await?;
                            let mut stream = channel.into_stream();
                            let _ = tokio::io::copy_bidirectional(&mut stream, &mut socket).await;
                        }
                        Err(e) => {
                            tracing::warn!("SOCKS5 目标连接失败: {e}");
                            socks5_reply(&mut socket, false).await?;
                        }
                    }
                    Ok::<(), crate::CoreError>(())
                };
                if let Err(e) = result.await {
                    tracing::debug!("SOCKS5 会话结束: {e}");
                }
            });
        }
    })
}
