// src-tauri/src/tray/events.rs
//! Tray event handlers
//!
//! This module handles all tray icon events including:
//! - Proxy group selection
//! - Mode changes (global/rule/direct)
//! - System proxy / TUN / config mixin toggles
//! - Geo data update
//! - Connection management
//! - Quit/restart operations

use crate::util::error::Result;
use tauri::menu::MenuEvent;
use tauri::{AppHandle, Emitter, Manager};
use tracing::{debug, error, info, warn};

/// Handle tray menu event
///
/// This function processes clicks on tray menu items and performs the
/// corresponding action. It handles:
/// - Proxy mode changes
/// - Proxy group selection
/// - System proxy / TUN / config mixin toggles
/// - Geo data update
/// - Connection management
/// - Quit/restart operations
///
/// # Arguments
///
/// * `app_handle` - Tauri app handle
/// * `event` - The menu event containing the clicked item ID
///
/// # Returns
///
/// `Result<()>` on success
pub async fn handle_tray_event(app_handle: &AppHandle, event: &MenuEvent) -> Result<()> {
    // NOTE: `MenuId` does not implement `Display`, use `as_ref()`.
    let item_id = event.id().as_ref().to_string();

    match item_id.as_str() {
        // Show / focus the main window
        "control_panel" => {
            if let Some(w) = app_handle.get_webview_window("main") {
                let _ = w.show();
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }

        // Proxy mode changes（mode_script 不是 mihomo 合法模式，菜单中已移除）
        "mode_global" | "mode_rule" | "mode_direct" => {
            info!("Tray: switching proxy mode to {}", item_id);
            let mode = item_id["mode_".len()..].to_string();
            let state = app_handle.state::<crate::AppState>();
            state.controller.apply_proxy_mode(app_handle, &mode).await?;
        }

        // System proxy toggle（真实系统代理，独立于 allow-lan）
        "system_proxy" => {
            info!("Tray: toggling system proxy");
            let state = app_handle.state::<crate::AppState>();
            let new_val = !state
                .config_manager
                .lock()
                .unwrap()
                .get_config()
                .general
                .system_proxy;
            state
                .controller
                .apply_system_proxy(app_handle, new_val)
                .await?;
        }

        // TUN mode toggle
        "tun_mode" => {
            info!("Tray: toggling TUN mode");
            let state = app_handle.state::<crate::AppState>();
            let new_val = !state.config_manager.lock().unwrap().get_config().tun.enable;
            state.controller.apply_tun(app_handle, new_val).await?;
        }

        // Config mixin toggle
        "config_mixin" => {
            info!("Tray: toggling config mixin");
            let state = app_handle.state::<crate::AppState>();
            // mixin_enabled 是应用级字段（不影响 runtime-config.yaml），
            // 切换不需要 reload mihomo，但仍要经 AppController 持事务锁串行，
            // 避免与 update_config / apply_* 等并发事务在 config_manager
            // 上交错（否则可能撞上正在 reload 的事务拿到中间态配置）。
            // 刷新托盘菜单勾选态 + 通知前端同步 UI 状态均在控制器事务内完成。
            state.controller.toggle_config_mixin(app_handle).await?;
        }

        // 开机自启开关（注册表 Run 键 → 根启动器 --clash-edge-autostart）
        "autostart" => {
            info!("Tray: toggling autostart");
            let new_val = !crate::util::autostart::get_autostart()?;
            crate::util::autostart::set_autostart(new_val)?;
            // 刷新菜单勾选态（refresh_tray 会带真实代理组重建，避免丢子菜单）
            crate::core::runtime::refresh_tray(app_handle).await?;
            // 通知前端同步「开机自启」开关状态（SettingsView）
            let _ = app_handle.emit(
                "autostart-changed",
                serde_json::json!({ "enable": new_val }),
            );
        }

        // Proxy group selection（ID 为不透明序号，真实名称查 tray/mod.rs 的映射）
        name if name.starts_with("proxy-item-") => {
            let Some((group, proxy)) = crate::tray::lookup_tray_menu_item(name) else {
                debug!("Tray: unknown proxy item id: {}", name);
                return Ok(());
            };
            // 节点名为空串 = 组本身（无子节点项），不触发 select。
            if proxy.is_empty() {
                debug!("Tray: proxy group {} has no selectable node", group);
                return Ok(());
            }
            info!("Tray: selecting proxy in group {}: {}", group, proxy);
            let state = app_handle.state::<crate::AppState>();
            let selected = {
                // 临界区内仅执行核心操作，块结束释放 guard 后再 refresh_tray/emit
                //（tokio Mutex 不可重入，refresh_tray 内部会再次 lock）。
                let core = state.core_manager.get();
                match core.as_ref() {
                    Some(c) => match c.select_proxy_group(group.clone(), proxy.clone()).await {
                        Ok(()) => Some((group, proxy)),
                        Err(e) => {
                            warn!("Failed to select proxy in group {}: {}", group, e);
                            None
                        }
                    },
                    None => None,
                }
            };
            if let Some((group, proxy)) = selected {
                // 托盘菜单勾选态跟随刷新
                let _ = crate::core::runtime::refresh_tray(app_handle).await;
                // 通知前端刷新代理组（ProxiesView 的选中态随之更新）。
                let _ = app_handle.emit(
                    "proxy-group-changed",
                    serde_json::json!({ "group": group, "proxy": proxy }),
                );
            }
        }

        // Geo data update
        "geodata_update" => {
            info!("Tray: initiating geo data update");
            let h = app_handle.clone();
            std::mem::drop(tauri::async_runtime::spawn(async move {
                if let Err(e) = crate::geodata::updater::update_geodata(&h).await {
                    error!("Geo data update failed: {}", e);
                }
            }));
        }

        // Close all connections
        // 与 restart 相同的锁纪律：在 tokio Mutex 临界区内跨 await 调用核心接口，
        // 块结束后释放 guard，避免影响其他需要锁的操作。
        "close_all" => {
            info!("Tray: closing all connections");
            let state = app_handle.state::<crate::AppState>();
            let core = state.core_manager.get();
            if let Some(c) = core.as_ref() {
                c.close_all_connections().await?;
            }
        }

        // Restart core
        "restart" => {
            info!("Tray: restarting core");
            let state = app_handle.state::<crate::AppState>();
            {
                // restart 在 tokio Mutex 临界区内执行（跨 await 持有 OK）；
                // 块结束后 guard 释放，refresh_tray 才安全（tokio Mutex 不可重入）。
                let core = state.core_manager.get();
                if let Some(c) = core.as_ref() {
                    c.restart().await?;
                }
            }
            // 重启后代理组菜单刷新（节点列表可能变化）。
            // core-status-changed 已由 start() 推送，前端同步刷新核心/代理组。
            crate::core::runtime::refresh_tray(app_handle).await?;
        }

        // Open dev tools for the main window（仅 debug 构建；release 不暴露调试面）
        #[cfg(debug_assertions)]
        "dev_tools" => {
            info!("Tray: opening dev tools");
            if let Some(w) = app_handle.get_webview_window("main") {
                w.open_devtools();
            }
        }

        // Force quit / quit
        "force_quit" | "quit" => {
            info!("Tray: quitting application");
            app_handle.exit(0);
        }

        _ => {
            debug!("Tray: unhandled menu item: {}", item_id);
        }
    }

    Ok(())
}
