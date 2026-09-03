//! 托盘:logo 图标 + 右下角状态点,按服务器分组的转发启停菜单。
//! 菜单构建只能同步读 tray_cache(回调上下文里不能 await);
//! 数据由 refresh_cache 在事件任务/command 里异步刷新
use crate::{AppState, TrayCache};
use ssh_tunnel_core::model::{ForwardKind, ForwardStatus, ServerStatus};
use tauri::image::Image;
use tauri::menu::{
    CheckMenuItemBuilder, Menu, MenuBuilder, MenuItemBuilder, PredefinedMenuItem, SubmenuBuilder,
};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager};

const TRAY_ID: &str = "main";

/// 托盘图标 = 应用 logo + 右下角状态点(绿=有转发在跑,红=有错误/重连,None=空闲)。
/// 纯代码叠加,不用为多状态维护多份图标资源
fn tray_icon(dot: Option<[u8; 4]>) -> Image<'static> {
    let img =
        Image::from_bytes(include_bytes!("../icons/32x32.png")).expect("内嵌托盘图标解码失败");
    let (w, h) = (img.width(), img.height());
    let mut data = img.rgba().to_vec();
    if let Some(rgba) = dot {
        draw_status_dot(&mut data, w, rgba);
    }
    Image::new_owned(data, w, h)
}

/// 右下角状态点:白圈描边保证在蓝色 logo 底上也清晰
fn draw_status_dot(data: &mut [u8], size: u32, rgba: [u8; 4]) {
    let c = size as f32 - 7.5;
    let r_outer = 6.5;
    let r_inner = 4.5;
    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 - c + 0.5;
            let dy = y as f32 - c + 0.5;
            let d2 = dx * dx + dy * dy;
            if d2 <= r_outer * r_outer {
                let i = ((y * size + x) * 4) as usize;
                if d2 <= r_inner * r_inner {
                    data[i..i + 4].copy_from_slice(&rgba);
                } else {
                    data[i..i + 4].copy_from_slice(&[255, 255, 255, 255]);
                }
            }
        }
    }
}

const GREEN: [u8; 4] = [76, 175, 80, 255];
const RED: [u8; 4] = [244, 67, 54, 255];

/// 从 manager 拉最新数据进托盘缓存(异步;在事件任务或 command 里调用)
pub async fn refresh_cache(app: &AppHandle) {
    let state = app.state::<AppState>();
    let servers = state.manager.list_servers().await;
    let forwards = state.manager.list_forwards().await;
    let snapshot = state.manager.snapshot().await;
    *state.tray_cache.write().unwrap() = TrayCache {
        servers,
        forwards,
        snapshot,
    };
}

fn forward_label(f: &ssh_tunnel_core::model::Forward) -> String {
    match f.kind {
        ForwardKind::Local => format!(
            "{} (本地 :{} → {}:{})",
            f.name,
            f.bind_port,
            f.target_host.as_deref().unwrap_or(""),
            f.target_port.unwrap_or(0)
        ),
        ForwardKind::Remote => format!(
            "{} (远程 :{} → {}:{})",
            f.name,
            f.bind_port,
            f.target_host.as_deref().unwrap_or(""),
            f.target_port.unwrap_or(0)
        ),
        ForwardKind::Dynamic => format!("{} (SOCKS5 :{})", f.name, f.bind_port),
    }
}

fn build_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let state = app.state::<AppState>();
    let cache = state.tray_cache.read().unwrap();

    let mut builder = MenuBuilder::new(app);
    if cache.servers.is_empty() {
        builder = builder.item(
            &MenuItemBuilder::with_id("empty", "暂无服务器,点击「显示主窗口」添加")
                .enabled(false)
                .build(app)?,
        );
    }
    for server in &cache.servers {
        let status = cache.snapshot.servers.get(&server.id).map(|s| s.status);
        let status_text = match status {
            Some(ServerStatus::Connected) => "已连接",
            Some(ServerStatus::Connecting) => "连接中…",
            Some(ServerStatus::Reconnecting) => "重连中…",
            Some(ServerStatus::Error) => "错误",
            _ => "未连接",
        };
        // 服务器节点用 Submenu 呈现,无需菜单 id(不可点击触发事件)
        let mut sub = SubmenuBuilder::new(app, format!("{} ({})", server.name, status_text));
        let server_forwards: Vec<_> = cache
            .forwards
            .iter()
            .filter(|f| f.server_id == server.id)
            .collect();
        if server_forwards.is_empty() {
            sub = sub.item(
                &MenuItemBuilder::with_id(format!("none:{}", server.id), "暂无转发")
                    .enabled(false)
                    .build(app)?,
            );
        }
        for f in server_forwards {
            let running = matches!(
                cache.snapshot.forwards.get(&f.id).map(|s| s.status),
                Some(ForwardStatus::Running)
            );
            sub = sub.item(
                &CheckMenuItemBuilder::with_id(format!("fwd:{}", f.id), forward_label(f))
                    .checked(running)
                    .build(app)?,
            );
        }
        sub = sub
            .separator()
            .item(&MenuItemBuilder::with_id(format!("add:{}", server.id), "添加转发…").build(app)?);
        builder = builder.item(&sub.build()?);
    }
    builder = builder
        .item(&PredefinedMenuItem::separator(app)?)
        .item(&MenuItemBuilder::with_id("show", "显示主窗口").build(app)?)
        .item(&MenuItemBuilder::with_id("quit", "退出").build(app)?);
    builder.build()
}

/// 状态点规则:任一错误/重连 → 红点;有转发在跑 → 绿点;否则纯 logo
fn overall_icon(app: &AppHandle) -> Image<'static> {
    let state = app.state::<AppState>();
    let cache = state.tray_cache.read().unwrap();
    let has_error = cache
        .snapshot
        .servers
        .values()
        .any(|s| s.status == ServerStatus::Error || s.status == ServerStatus::Reconnecting)
        || cache
            .snapshot
            .forwards
            .values()
            .any(|s| s.status == ForwardStatus::Error);
    let has_running = cache
        .snapshot
        .forwards
        .values()
        .any(|s| s.status == ForwardStatus::Running);
    let dot = if has_error {
        Some(RED)
    } else if has_running {
        Some(GREEN)
    } else {
        None
    };
    tray_icon(dot)
}

/// 从最新缓存重建菜单 + 更新图标(同步;由事件任务与 command 在 refresh_cache 后调用)
pub fn refresh_tray(app: &AppHandle) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    if let Ok(menu) = build_menu(app) {
        let _ = tray.set_menu(Some(menu));
    }
    let _ = tray.set_icon(Some(overall_icon(app)));
}

pub fn build_tray(app: &mut tauri::App) -> tauri::Result<()> {
    let menu = build_menu(app.handle())?;
    TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("SSH Tunnel")
        .icon(tray_icon(None))
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            let id = event.id().0.as_str();
            if let Some(fwd_id) = id.strip_prefix("fwd:") {
                toggle_forward(app, fwd_id);
            } else if let Some(server_id) = id.strip_prefix("add:") {
                show_window(app);
                let _ = app.emit(
                    "navigate",
                    serde_json::json!({ "view": "add-forward", "server_id": server_id }),
                );
            } else if id == "show" {
                show_window(app);
            } else if id == "quit" {
                let state = app.state::<AppState>();
                let manager = state.manager.clone();
                // 先优雅关停所有连接再退出
                tauri::async_runtime::spawn(async move {
                    manager.shutdown_all().await;
                    std::process::exit(0);
                });
            }
        })
        .build(app)?;
    Ok(())
}

fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn toggle_forward(app: &AppHandle, forward_id: &str) {
    let state = app.state::<AppState>();
    let manager = state.manager.clone();
    let forward_id = forward_id.to_string();
    let running = {
        let cache = state.tray_cache.read().unwrap();
        matches!(
            cache.snapshot.forwards.get(&forward_id).map(|s| s.status),
            Some(ForwardStatus::Running) | Some(ForwardStatus::Starting)
        )
    };
    tauri::async_runtime::spawn(async move {
        let result = if running {
            manager.stop_forward(&forward_id).await
        } else {
            manager.start_forward(&forward_id).await
        };
        if let Err(e) = result {
            tracing::error!("切换转发失败: {e}");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_icon_is_logo_size() {
        let img = tray_icon(None);
        assert_eq!((img.width(), img.height()), (32, 32));
    }

    #[test]
    fn status_dot_overlay() {
        let img = tray_icon(Some(GREEN));
        let data = img.rgba();
        let px = |x: u32, y: u32| {
            let i = ((y * 32 + x) * 4) as usize;
            &data[i..i + 4]
        };
        // 圆心(右下角)是状态色
        assert_eq!(px(25, 25), &GREEN);
        // 外圈描边是白色(距圆心 6px,在外圈与内圈之间)
        assert_eq!(px(30, 24), &[255, 255, 255, 255]);
        // 无状态点时图标保持 logo 原样(逐字节一致)
        assert_eq!(tray_icon(None).rgba(), tray_icon(None).rgba());
    }
}
