pub mod actor;
pub mod client;
pub mod manager;

use crate::model::{ForwardStatus, ServerStatus};
use serde::Serialize;

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
