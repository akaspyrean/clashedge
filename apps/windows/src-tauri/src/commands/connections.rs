// src-tauri/src/commands/connections.rs
//! 连接命令：获取活动连接、关闭全部连接

use crate::util::error::Result;
use tauri::{command, Manager};

#[command]
pub async fn get_connections(app: tauri::AppHandle) -> Result<serde_json::Value> {
    let state = app.state::<crate::AppState>();
    let core = state.core_manager.get();
    if let Some(core) = core.as_ref() {
        core.get_connections().await
    } else {
        Err(crate::util::Error::InvalidState(
            "Core not initialized".to_string(),
        ))
    }
}

#[command]
pub async fn close_all_connections(app: tauri::AppHandle) -> Result<()> {
    let state = app.state::<crate::AppState>();
    let core = state.core_manager.get();
    if let Some(core) = core.as_ref() {
        core.close_all_connections().await
    } else {
        Err(crate::util::Error::InvalidState(
            "Core not initialized".to_string(),
        ))
    }
}
