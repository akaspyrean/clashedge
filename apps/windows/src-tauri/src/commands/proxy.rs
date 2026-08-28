// src-tauri/src/commands/proxy.rs
//! 代理命令：系统代理、TUN 模式、代理模式、延迟测试、代理组
//!
//! 三个开关类命令不再直接操作 CoreManager 或注册表，而是路由到统一编排层
//! `core::runtime`（校验 → 持久化 → 同步运行时 → 实时应用 → 回滚 → 通知）。
//! 这样前端命令与托盘事件行为一致，避免两套逻辑漂移。

use tauri::{command, AppHandle};

use crate::util::error::Result;

#[command]
pub async fn set_system_proxy(app: AppHandle, enable: bool) -> Result<()> {
    crate::core::runtime::apply_system_proxy(&app, enable).await
}

#[command]
pub async fn set_tun_mode(app: AppHandle, enable: bool) -> Result<()> {
    crate::core::runtime::apply_tun(&app, enable).await
}

#[command]
pub async fn set_proxy_mode(app: AppHandle, mode: String) -> Result<()> {
    crate::core::runtime::apply_proxy_mode(&app, &mode).await
}

#[command]
pub async fn test_proxy_latency(
    state: tauri::State<'_, crate::AppState>,
    group: String,
    url: Option<String>,
) -> Result<Vec<serde_json::Value>> {
    let core_guard = state.core_manager.get();
    if let Some(core) = core_guard.as_ref() {
        core.test_proxy_latency(group, url).await
    } else {
        Err(crate::util::error::Error::InvalidState(
            "Core not initialized".to_string(),
        ))
    }
}

#[command]
pub async fn get_proxy_groups(
    state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<serde_json::Value>> {
    let core_guard = state.core_manager.get();
    if let Some(core) = core_guard.as_ref() {
        core.get_proxy_groups().await
    } else {
        Ok(vec![])
    }
}

#[command]
pub async fn select_proxy_group(
    app: AppHandle,
    state: tauri::State<'_, crate::AppState>,
    group: String,
    proxy: String,
) -> Result<()> {
    {
        // 临界区内仅执行核心操作，块结束释放 guard 后再 refresh_tray
        //（tokio Mutex 不可重入，refresh_tray 内部会再次 lock）。
        let core_guard = state.core_manager.get();
        match core_guard.as_ref() {
            Some(core) => core.select_proxy_group(group, proxy).await?,
            None => {
                return Err(crate::util::error::Error::InvalidState(
                    "Core not initialized".to_string(),
                ))
            }
        }
    }
    // UI 切换节点后，托盘对应子菜单勾选态跟随刷新。
    crate::core::runtime::refresh_tray(&app).await
}
