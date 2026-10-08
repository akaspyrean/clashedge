// src-tauri/src/commands/core.rs
//! 核心命令：核心服务启动/停止/重启/状态

use crate::core::app_controller::CoreOp;
use crate::util::error::Result;
use tauri::{command, AppHandle, Manager, State};

#[command]
pub async fn get_status(state: State<'_, crate::AppState>) -> Result<serde_json::Value> {
    let core_guard = state.core_manager.get();
    if let Some(core) = core_guard.as_ref() {
        Ok(core.get_status().await)
    } else {
        Ok(serde_json::json!({"running": false}))
    }
}

#[command]
pub async fn start_core(app: AppHandle, state: State<'_, crate::AppState>) -> Result<()> {
    state.controller.core_op(&app, CoreOp::Start).await
}

#[command]
pub async fn stop_core(app: AppHandle) -> Result<()> {
    // 统一编排：停核心 + 关闭系统代理（config/registry/journal/事件/托盘）
    // 全部走 AppController::stop_core_and_sync_proxy，不绕过配置事务。
    let state = app.state::<crate::AppState>();
    state.controller.stop_core_and_sync_proxy(&app).await
}

#[command]
pub async fn restart_core(app: AppHandle, state: State<'_, crate::AppState>) -> Result<()> {
    state.controller.core_op(&app, CoreOp::Restart).await
}

#[command]
pub async fn reload_config(app: AppHandle, state: State<'_, crate::AppState>) -> Result<()> {
    state.controller.core_op(&app, CoreOp::Reload).await
}
