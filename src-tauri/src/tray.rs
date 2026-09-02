//! 托盘占位:Task 13 填实
use tauri::AppHandle;

pub async fn refresh_cache(_app: &AppHandle) {}

pub fn refresh_tray(_app: &AppHandle) {}

pub fn build_tray(_app: &mut tauri::App) -> tauri::Result<()> {
    Ok(())
}
