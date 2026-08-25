// src-tauri/src/commands/tray.rs
//! 托盘命令：获取/更新托盘菜单状态

use crate::util::error::Result;
use tauri::{command, State};

#[command]
pub async fn get_tray_menu_state(state: State<'_, crate::AppState>) -> Result<serde_json::Value> {
    let core_guard = state.core_manager.get();
    if let Some(core) = core_guard.as_ref() {
        // Return core status and basic tray state
        Ok(serde_json::json!({
            "status": core.status().to_string(),
            "is_running": core.is_running(),
        }))
    } else {
        Ok(serde_json::json!({}))
    }
}

#[command]
pub async fn update_tray_menu(state: State<'_, crate::AppState>) -> Result<()> {
    // 复用编排层刷新逻辑（带真实代理组重建菜单，避免空代理组丢子菜单）。
    let app = {
        let tray_guard = state.tray.lock().unwrap();
        match tray_guard.as_ref() {
            Some(tray) => tray.app_handle().clone(),
            None => return Ok(()),
        }
    };
    crate::core::runtime::refresh_tray(&app).await
}
