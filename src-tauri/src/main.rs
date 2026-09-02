mod commands;
mod logging;
mod tray;

use ssh_tunnel_core::config::ConfigStore;
use ssh_tunnel_core::known_hosts::KnownHosts;
use ssh_tunnel_core::model::{Forward, Server, Settings};
use ssh_tunnel_core::paths;
use ssh_tunnel_core::secrets::KeyringStore;
use ssh_tunnel_core::ssh::client::{HostKeyDecider, HostKeyInfo};
use ssh_tunnel_core::ssh::manager::SshManager;
use ssh_tunnel_core::ssh::StatusSnapshot;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tauri::{Emitter, Manager};
use tokio::sync::{broadcast, oneshot, Mutex};

/// 托盘的同步数据源:托盘菜单构建发生在任意回调上下文,
/// 不能 await(在 runtime 线程里 block_on 会 panic),故用 RwLock 缓存,
/// 由事件任务与 CRUD command 主动刷新
#[derive(Default)]
pub struct TrayCache {
    pub servers: Vec<Server>,
    pub forwards: Vec<Forward>,
    pub snapshot: StatusSnapshot,
}

pub struct AppState {
    pub manager: Arc<SshManager>,
    pub pending_host_keys: Arc<Mutex<HashMap<String, oneshot::Sender<bool>>>>,
    pub settings_cache: RwLock<Settings>,
    pub tray_cache: RwLock<TrayCache>,
    pub logs: logging::LogBuffer,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let logs = logging::init_logging();

    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![]),
        ))
        .setup(move |app| {
            let dir = paths::config_dir();
            std::fs::create_dir_all(&dir)?;
            let store = ConfigStore::new(dir.join("config.json"));
            let known_hosts = Arc::new(Mutex::new(KnownHosts::new(dir.join("known_hosts"))));
            let pending: Arc<Mutex<HashMap<String, oneshot::Sender<bool>>>> = Arc::new(Mutex::new(HashMap::new()));

            // host key 决策回路:core 回调 → tauri event → 前端弹窗 → respond_host_key command。
            // pending map 与 AppState 共享同一个 Arc,respond_host_key 才能找到回调
            let app_handle = app.handle().clone();
            let pending_for_decider = pending.clone();
            let decider: HostKeyDecider = Arc::new(move |info: HostKeyInfo| {
                let app = app_handle.clone();
                let pending = pending_for_decider.clone();
                Box::pin(async move {
                    let (tx, rx) = oneshot::channel();
                    let prompt_id = uuid::Uuid::new_v4().to_string();
                    pending.lock().await.insert(prompt_id.clone(), tx);
                    let payload = serde_json::json!({
                        "prompt_id": prompt_id,
                        "host": info.host,
                        "port": info.port,
                        "fingerprint": info.fingerprint,
                        "is_mismatch": info.is_mismatch,
                    });
                    if app.emit("host-key-prompt", payload).is_err() {
                        return false;
                    }
                    rx.await.unwrap_or(false)
                })
            });

            // SshManager::new 内部 tokio::spawn(快照跟随任务),必须在 runtime 上下文里构造,
            // 否则主线程直接调用会 panic("no reactor running");
            // setup 是同步上下文,此处 block_on 安全(不在 runtime worker 内)
            let (manager, settings) = tauri::async_runtime::block_on(async {
                let manager = SshManager::new(store, Arc::new(KeyringStore::new()), known_hosts, decider)?;
                let settings = manager.settings().await;
                Ok::<_, ssh_tunnel_core::CoreError>((manager, settings))
            })
            .map_err(|e| format!("加载配置失败: {e}"))?;
            let manager = Arc::new(manager);

            // 挂上 AppHandle 后新日志实时推给前端(挂载前的由 get_logs 首屏补齐)
            logs.attach_app(app.handle().clone());

            app.manage(AppState {
                manager: manager.clone(),
                pending_host_keys: pending,
                settings_cache: RwLock::new(settings),
                tray_cache: RwLock::new(TrayCache::default()),
                logs,
            });

            // core 事件 → 前端 + 托盘缓存 + 托盘菜单
            let mut rx = manager.subscribe();
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                tray::refresh_cache(&app_handle).await;
                tray::refresh_tray(&app_handle);
                // 事件突发时广播通道可能滞后:Lagged 只是丢帧(快照类事件天然幂等,
                // 下一帧会覆盖),while-let 写法会让任务静默退出,前端与托盘从此冻结
                loop {
                    match rx.recv().await {
                        Ok(ev) => {
                            let _ = app_handle.emit("tunnel-event", &ev);
                            tray::refresh_cache(&app_handle).await;
                            tray::refresh_tray(&app_handle);
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
            });

            tray::build_tray(app)?;

            // 恢复 auto_start 转发
            tauri::async_runtime::spawn(async move {
                manager.start_auto_forwards().await;
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_servers,
            commands::upsert_server,
            commands::delete_server,
            commands::list_forwards,
            commands::upsert_forward,
            commands::delete_forward,
            commands::start_forward,
            commands::stop_forward,
            commands::connect_server,
            commands::disconnect_server,
            commands::get_snapshot,
            commands::get_settings,
            commands::save_settings,
            commands::get_logs,
            commands::respond_host_key,
        ])
        .on_window_event(|window, event| {
            // 关闭主窗口 = 最小化到托盘(可在设置中关闭此行为)
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let minimize = window
                    .state::<AppState>()
                    .settings_cache
                    .read()
                    .map(|s| s.minimize_to_tray)
                    .unwrap_or(true);
                if minimize {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("运行 tauri 应用失败");
}

fn main() {
    run()
}
