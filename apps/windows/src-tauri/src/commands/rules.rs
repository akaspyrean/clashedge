// src-tauri/src/commands/rules.rs
//! 规则命令：获取当前运行配置的规则列表（只读）

use crate::util::error::Result;
use tauri::{command, Manager};

#[command]
pub async fn get_rules(app: tauri::AppHandle) -> Result<Vec<serde_json::Value>> {
    let state = app.state::<crate::AppState>();
    let core = state.core_manager.get();
    if let Some(core) = core.as_ref() {
        core.get_rules().await
    } else {
        Err(crate::util::Error::InvalidState(
            "Core not initialized".to_string(),
        ))
    }
}
