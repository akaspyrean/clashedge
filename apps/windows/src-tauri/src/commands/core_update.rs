// src-tauri/src/commands/core_update.rs
//! 独立 Mihomo 核心更新：签名检查、下载与版本化暂存。

use tauri::{command, AppHandle};

use crate::util::error::Result;

#[command]
pub fn core_update_chain_ready() -> bool {
    crate::update::core::chain_ready()
}

#[command]
pub async fn check_core_update(app: AppHandle) -> Result<crate::update::core::CoreUpdateStatus> {
    crate::update::core::check_for_core_update(&app).await
}

#[command]
pub async fn download_core_update(app: AppHandle) -> Result<crate::update::core::StagedCoreUpdate> {
    crate::update::core::download_core_update(&app).await
}

#[command]
pub async fn get_staged_core_update(
    app: AppHandle,
) -> Result<Option<crate::update::core::StagedCoreUpdate>> {
    crate::update::core::staged_core_update(&app).await
}
