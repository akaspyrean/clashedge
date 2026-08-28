// src-tauri/src/commands/core.rs
//! 核心命令：核心服务启动/停止/重启/状态

use crate::util::error::Result;
use tauri::{command, AppHandle, State};

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
    {
        // 临界区内仅启动核心，块结束释放 guard 后再 refresh_tray
        //（tokio Mutex 不可重入，refresh_tray 内部会再次 lock）。
        let core_guard = state.core_manager.get();
        match core_guard.as_ref() {
            Some(core) => core.start().await?,
            None => {
                return Err(crate::util::Error::InvalidState(
                    "Core not initialized".to_string(),
                ))
            }
        }
    }
    // 核心启动后托盘菜单（运行态图标/代理组子菜单）跟随刷新。
    crate::core::runtime::refresh_tray(&app).await
}

#[command]
pub async fn stop_core(app: AppHandle) -> Result<()> {
    // 统一编排：停核心 + 关闭系统代理（config/registry/journal/事件/托盘）
    // 全部走 runtime::stop_core_and_sync_proxy，不再绕过 config_tx 事务。
    crate::core::runtime::stop_core_and_sync_proxy(&app).await
}

#[command]
pub async fn restart_core(app: AppHandle, state: State<'_, crate::AppState>) -> Result<()> {
    {
        let core_guard = state.core_manager.get();
        match core_guard.as_ref() {
            Some(core) => core.restart().await?,
            None => {
                return Err(crate::util::Error::InvalidState(
                    "Core not initialized".to_string(),
                ))
            }
        }
    }
    // 重启后托盘菜单（代理组子菜单节点可能变化）跟随刷新。
    crate::core::runtime::refresh_tray(&app).await
}

#[command]
pub async fn reload_config(app: AppHandle, state: State<'_, crate::AppState>) -> Result<()> {
    {
        let core_guard = state.core_manager.get();
        match core_guard.as_ref() {
            Some(core) => core.reload_config().await?,
            None => {
                return Err(crate::util::Error::InvalidState(
                    "Core not initialized".to_string(),
                ))
            }
        }
    }
    // 重载后托盘菜单（勾选态/代理组）跟随刷新。
    crate::core::runtime::refresh_tray(&app).await
}
