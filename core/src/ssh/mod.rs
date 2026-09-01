pub mod actor;
pub mod client;
pub mod manager;

use crate::model::{ForwardStatus, ServerStatus};
use serde::Serialize;
use std::collections::HashMap;

/// actor → 外部（manager/Tauri 前端）的状态事件，serde tag 形式便于前端 match
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TunnelEvent {
    ServerStatus {
        server_id: String,
        status: ServerStatus,
        error: Option<String>,
    },
    ForwardStatus {
        forward_id: String,
        server_id: String,
        status: ForwardStatus,
        error: Option<String>,
    },
}

/// 服务器状态快照条目：当前状态 + 最近一次错误（供托盘与前端首屏渲染）
#[derive(Debug, Clone, Serialize)]
pub struct ServerStatusEntry {
    pub status: ServerStatus,
    pub error: Option<String>,
}

/// 转发状态快照条目：当前状态 + 最近一次错误
#[derive(Debug, Clone, Serialize)]
pub struct ForwardStatusEntry {
    pub status: ForwardStatus,
    pub error: Option<String>,
}

/// 全局状态快照，由 SshManager 跟随 TunnelEvent 流维护
#[derive(Debug, Clone, Default, Serialize)]
pub struct StatusSnapshot {
    pub servers: HashMap<String, ServerStatusEntry>,
    pub forwards: HashMap<String, ForwardStatusEntry>,
}
