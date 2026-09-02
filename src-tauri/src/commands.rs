use crate::logging::LogEntry;
use crate::{tray, AppState};
use ssh_tunnel_core::model::{Forward, Server, Settings};
use ssh_tunnel_core::secrets::SecretKind;
use ssh_tunnel_core::ssh::StatusSnapshot;
use tauri::{AppHandle, State};

#[derive(Debug, serde::Deserialize)]
pub struct UpsertServerInput {
    pub server: Server,
    /// Some = 写入钥匙串;None = 保持已有值不变
    pub password: Option<String>,
    pub key_data: Option<String>,
    pub key_passphrase: Option<String>,
}

fn err(e: ssh_tunnel_core::CoreError) -> String {
    e.to_string()
}

/// 配置类变更后刷新托盘缓存与菜单(状态类变更走事件回路,见 main.rs)
async fn after_config_change(app: &AppHandle) {
    tray::refresh_cache(app).await;
    tray::refresh_tray(app);
}

#[tauri::command]
pub async fn list_servers(state: State<'_, AppState>) -> Result<Vec<Server>, String> {
    Ok(state.manager.list_servers().await)
}

#[tauri::command]
pub async fn upsert_server(input: UpsertServerInput, app: AppHandle, state: State<'_, AppState>) -> Result<Server, String> {
    // 认证方式变更时旧凭据作废,先清再写
    let old = state.manager.list_servers().await.into_iter().find(|s| s.id == input.server.id);
    if let Some(old) = old {
        if old.auth != input.server.auth {
            for kind in [SecretKind::Password, SecretKind::Key, SecretKind::KeyPassphrase] {
                let _ = state.manager.secrets().delete(&input.server.id, kind);
            }
        }
    }
    let saved = state.manager.upsert_server(input.server).await.map_err(err)?;
    if let Some(v) = input.password {
        state.manager.secrets().set(&saved.id, SecretKind::Password, &v).map_err(err)?;
    }
    if let Some(v) = input.key_data {
        state.manager.secrets().set(&saved.id, SecretKind::Key, &v).map_err(err)?;
    }
    if let Some(v) = input.key_passphrase {
        state.manager.secrets().set(&saved.id, SecretKind::KeyPassphrase, &v).map_err(err)?;
    }
    after_config_change(&app).await;
    Ok(saved)
}

#[tauri::command]
pub async fn delete_server(id: String, app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    state.manager.delete_server(&id).await.map_err(err)?;
    after_config_change(&app).await;
    Ok(())
}

#[tauri::command]
pub async fn list_forwards(state: State<'_, AppState>) -> Result<Vec<Forward>, String> {
    Ok(state.manager.list_forwards().await)
}

#[tauri::command]
pub async fn upsert_forward(forward: Forward, app: AppHandle, state: State<'_, AppState>) -> Result<Forward, String> {
    let saved = state.manager.upsert_forward(forward).await.map_err(err)?;
    after_config_change(&app).await;
    Ok(saved)
}

#[tauri::command]
pub async fn delete_forward(id: String, app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    state.manager.delete_forward(&id).await.map_err(err)?;
    after_config_change(&app).await;
    Ok(())
}

#[tauri::command]
pub async fn start_forward(id: String, state: State<'_, AppState>) -> Result<(), String> {
    state.manager.start_forward(&id).await.map_err(err)
}

#[tauri::command]
pub async fn stop_forward(id: String, state: State<'_, AppState>) -> Result<(), String> {
    state.manager.stop_forward(&id).await.map_err(err)
}

#[tauri::command]
pub async fn connect_server(id: String, state: State<'_, AppState>) -> Result<(), String> {
    state.manager.connect_server(&id).await.map_err(err)
}

#[tauri::command]
pub async fn disconnect_server(id: String, state: State<'_, AppState>) -> Result<(), String> {
    state.manager.disconnect_server(&id).await.map_err(err)
}

#[tauri::command]
pub async fn get_snapshot(state: State<'_, AppState>) -> Result<StatusSnapshot, String> {
    Ok(state.manager.snapshot().await)
}

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> Result<Settings, String> {
    Ok(state.manager.settings().await)
}

#[tauri::command]
pub async fn save_settings(settings: Settings, app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    state.manager.update_settings(settings.clone()).await.map_err(err)?;
    *state.settings_cache.write().unwrap() = settings.clone();
    // 开机自启跟随设置
    use tauri_plugin_autostart::ManagerExt;
    let autostart = app.autolaunch();
    let _ = if settings.launch_at_login { autostart.enable() } else { autostart.disable() };
    Ok(())
}

#[tauri::command]
pub async fn get_logs(state: State<'_, AppState>) -> Result<Vec<LogEntry>, String> {
    Ok(state.logs.snapshot())
}

#[tauri::command]
pub async fn respond_host_key(prompt_id: String, trust: bool, state: State<'_, AppState>) -> Result<(), String> {
    let sender = state.pending_host_keys.lock().await.remove(&prompt_id);
    if let Some(tx) = sender {
        let _ = tx.send(trust);
    }
    Ok(())
}
