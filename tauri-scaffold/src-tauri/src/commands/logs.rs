// src-tauri/src/commands/logs.rs
//! 日志流命令：启动 / 停止 后端 → mihomo `/logs` SSE 转发任务。
//!
//! 前端日志页挂载时调用 `start_log_stream`，卸载时调用 `stop_log_stream`。
//! 任务句柄存在 AppState.log_stream（std Mutex，abort 为同步调用）。

use tauri::{command, AppHandle, Manager};

use crate::util::error::Result;

#[command]
pub async fn start_log_stream(app: AppHandle) -> Result<()> {
    let state = app.state::<crate::AppState>();

    // 取消上一次遗留的任务（重复进入日志页时避免残留多条连接）
    if let Some(handle) = state.log_stream.lock().unwrap().take() {
        handle.abort();
    }

    let (controller, secret) = {
        let cfg = state.config_manager.lock().unwrap().get_config();
        (cfg.proxy.external_controller.clone(), cfg.proxy.secret.clone())
    };

    let handle = crate::core::logs::spawn_log_stream(app.clone(), &controller, &secret);
    *state.log_stream.lock().unwrap() = Some(handle);
    Ok(())
}

#[command]
pub async fn stop_log_stream(app: AppHandle) -> Result<()> {
    let state = app.state::<crate::AppState>();
    if let Some(handle) = state.log_stream.lock().unwrap().take() {
        handle.abort();
    }
    Ok(())
}
