// src-tauri/src/commands/core.rs
//! 核心命令：核心服务启动/停止/重启/状态

use crate::util::error::Result;
use tauri::{command, AppHandle, Emitter, Manager, State};

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
pub async fn stop_core(app: AppHandle, state: State<'_, crate::AppState>) -> Result<()> {
    {
        let core_guard = state.core_manager.get();
        if let Some(core) = core_guard.as_ref() {
            core.stop().await?;
        }
    }

    // 内核停止后，若系统代理仍指向本应用端口，必须立即关闭——
    // 否则系统代理继续指向已死的 127.0.0.1:7890，用户全网 ERR_CONNECTION_REFUSED。
    // 配置里的 system_proxy 意图也同步置 false，避免下次启动前被误还原。
    let was_on = {
        let state = app.state::<crate::AppState>();
        let mut cfg_mgr = state.config_manager.lock().unwrap();
        let mut cfg = cfg_mgr.get_config();
        let was = cfg.general.system_proxy;
        if was {
            cfg.general.system_proxy = false;
            let _ = cfg_mgr.set_config(cfg);
        }
        was
    };
    if was_on {
        if let Err(e) = crate::proxy::system_proxy::set_system_proxy(false, "", &[], None) {
            tracing::warn!("Failed to clear system proxy after stopping core: {}", e);
        }
        let _ = app.emit(
            "system-proxy-changed",
            serde_json::json!({ "enable": false }),
        );
    }
    // 停止后托盘菜单（运行态图标/代理组子菜单）跟随刷新。
    crate::core::runtime::refresh_tray(&app).await
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
