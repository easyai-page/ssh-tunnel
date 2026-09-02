//! 极简 SOCKS5 服务端：仅无认证 + CONNECT,够浏览器/git 等客户端用
use crate::CoreError;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

pub async fn socks5_accept_target(stream: &mut TcpStream) -> Result<(String, u16), CoreError> {
    let mut head = [0u8; 2];
    stream.read_exact(&mut head).await?;
    if head[0] != 0x05 {
        return Err(CoreError::Other("非 SOCKS5 握手".into()));
    }
    // 读方法列表并选择无认证(0x00)
    let mut methods = vec![0u8; head[1] as usize];
    stream.read_exact(&mut methods).await?;
    stream.write_all(&[0x05, 0x00]).await?;

    let mut req = [0u8; 4];
    stream.read_exact(&mut req).await?;
    if req[0] != 0x05 || req[1] != 0x01 {
        // REP=0x07: 命令不支持
        stream
            .write_all(&[0x05, 0x07, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
            .await?;
        return Err(CoreError::Other("SOCKS5 仅支持 CONNECT".into()));
    }
    let host = match req[3] {
        0x01 => {
            let mut b = [0u8; 4];
            stream.read_exact(&mut b).await?;
            std::net::Ipv4Addr::from(b).to_string()
        }
        0x03 => {
            let mut len = [0u8; 1];
            stream.read_exact(&mut len).await?;
            let mut b = vec![0u8; len[0] as usize];
            stream.read_exact(&mut b).await?;
            String::from_utf8(b).map_err(|_| CoreError::Other("SOCKS5 域名非 UTF-8".into()))?
        }
        0x04 => {
            let mut b = [0u8; 16];
            stream.read_exact(&mut b).await?;
            std::net::Ipv6Addr::from(b).to_string()
        }
        _ => return Err(CoreError::Other("SOCKS5 未知地址类型".into())),
    };
    let mut port = [0u8; 2];
    stream.read_exact(&mut port).await?;
    Ok((host, u16::from_be_bytes(port)))
}

pub async fn socks5_reply(stream: &mut TcpStream, success: bool) -> Result<(), CoreError> {
    let rep = if success { 0x00 } else { 0x05 };
    stream
        .write_all(&[0x05, rep, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
        .await?;
    Ok(())
}
