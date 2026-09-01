//! 远程转发(-R):请求服务器监听,服务器侧来连接时经 forwarded-tcpip 通道回到客户端,
//! 由 ClientHandler::server_channel_open_forwarded_tcpip 按端口查 remote_forwards 桥接到本地目标
use crate::model::Forward;
use crate::ssh::client::{ClientHandler, RemoteTarget};
use crate::CoreError;
use russh::client;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// 返回服务器实际分配的端口（bind_port=0 时由服务器选定），调用方应写回 forward 副本
pub async fn start_remote_forward(
    forward: &Forward,
    handle: &client::Handle<ClientHandler>,
    remote_forwards: &Arc<RwLock<HashMap<u32, RemoteTarget>>>,
) -> Result<u32, CoreError> {
    let target = RemoteTarget {
        forward_id: forward.id.clone(),
        target_host: forward.target_host.clone().unwrap_or_else(|| "127.0.0.1".into()),
        target_port: forward.target_port.ok_or_else(|| CoreError::Other("远程转发缺少目标端口".into()))?,
    };
    let assigned = handle.tcpip_forward(forward.bind_addr.clone(), forward.bind_port as u32).await?;
    remote_forwards.write().await.insert(assigned, target);
    Ok(assigned)
}

/// 无条件清理本地映射：cancel 失败（如连接已断）只记 warn，不传播错误，
/// 否则映射残留会让重连后的 forwarded-tcpip 分发到错误目标
pub async fn stop_remote_forward(
    forward: &Forward,
    handle: &client::Handle<ClientHandler>,
    remote_forwards: &Arc<RwLock<HashMap<u32, RemoteTarget>>>,
) -> Result<(), CoreError> {
    if let Err(e) = handle.cancel_tcpip_forward(forward.bind_addr.clone(), forward.bind_port as u32).await {
        tracing::warn!("cancel_tcpip_forward 失败(仍清理本地映射): {e}");
    }
    // 按 forward_id 清理,兼容 bind_port=0(分配端口)的情况
    remote_forwards.write().await.retain(|_, t| t.forward_id != forward.id);
    Ok(())
}
