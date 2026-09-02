//! 日志三去向:stdout、滚动文件、前端事件
use serde::Serialize;
use ssh_tunnel_core::paths;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
use tracing_subscriber::Registry;

#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    pub timestamp: String,
    pub level: String,
    pub message: String,
}

const MAX_LOGS: usize = 500;

#[derive(Clone, Default)]
pub struct LogBuffer {
    inner: Arc<Mutex<VecDeque<LogEntry>>>,
    /// app 创建前 tracing 全局层就得上岗,AppHandle 只能事后挂;
    /// 挂载前的日志仍在缓冲区,前端 get_logs 首屏拉取补齐
    app: Arc<Mutex<Option<AppHandle>>>,
}

impl LogBuffer {
    pub fn push(&self, entry: LogEntry) {
        {
            let mut logs = self.inner.lock().unwrap();
            if logs.len() >= MAX_LOGS {
                logs.pop_front();
            }
            logs.push_back(entry.clone());
        }
        // 实时推给前端日志面板;emit 失败(窗口未建/已毁)不影响落盘与缓冲。
        // 先克隆 AppHandle 出锁再 emit:emit 走 webview IPC 可能阻塞,
        // 持锁调用会让其他线程的 push 全部排队
        let app = self.app.lock().unwrap().clone();
        if let Some(app) = app {
            let _ = app.emit("log", &entry);
        }
    }
    pub fn snapshot(&self) -> Vec<LogEntry> {
        self.inner.lock().unwrap().iter().cloned().collect()
    }
    pub fn attach_app(&self, app: AppHandle) {
        *self.app.lock().unwrap() = Some(app);
    }
}

struct BufferLayer {
    buffer: LogBuffer,
}

impl<S: tracing::Subscriber> Layer<S> for BufferLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let mut message = String::new();
        let mut visitor = MessageVisitor(&mut message);
        event.record(&mut visitor);
        self.buffer.push(LogEntry {
            timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
            level: event.metadata().level().to_string(),
            message,
        });
    }
}

struct MessageVisitor<'a>(&'a mut String);

impl tracing::field::Visit for MessageVisitor<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            *self.0 = format!("{value:?}");
        }
    }
}

pub fn init_logging() -> LogBuffer {
    let buffer = LogBuffer::default();
    let log_dir = paths::config_dir().join("logs");
    std::fs::create_dir_all(&log_dir).ok();
    let file = tracing_appender::rolling::daily(&log_dir, "ssh-tunnel.log");
    tracing::subscriber::set_global_default(
        Registry::default()
            .with(BufferLayer {
                buffer: buffer.clone(),
            })
            .with(tracing_subscriber::fmt::layer().with_writer(std::io::stdout))
            .with(
                tracing_subscriber::fmt::layer()
                    .with_ansi(false)
                    .with_writer(file),
            ),
    )
    .expect("初始化日志失败");
    buffer
}
