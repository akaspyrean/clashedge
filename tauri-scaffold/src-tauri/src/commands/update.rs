// src-tauri/src/commands/update.rs
//! Portable Updater 命令：检查 / 下载暂存 / 查询暂存状态 / 清除暂存
//!
//! 应用侧只负责「取到已验签的更新包并暂存」；最终替换由根启动器在下次
//! 启动时执行（Windows 无法覆盖运行中的自身映像）。

use crate::update::{self, UpdateStatus};
use crate::util::error::Result;
use tauri::command;

#[command]
pub async fn check_update(app: tauri::AppHandle) -> Result<serde_json::Value> {
    match update::check_for_update(&app).await {
        Ok(status) => serde_json::to_value(status)
            .map_err(|e| crate::util::error::Error::Other(e.to_string())),
        Err(e) => Err(e),
    }
}

#[command]
pub async fn download_update(
    app: tauri::AppHandle,
    version: String,
    url: String,
    sha256: String,
) -> Result<crate::update::PendingUpdate> {
    let manifest = update::UpdateManifest {
        version,
        url,
        sha256,
        notes: String::new(),
    };
    // 下载前复验 URL（前端传参不可信）
    crate::util::fetch::validate_url(&manifest.url).await?;
    update::download_and_stage(&app, &manifest).await
}

#[command]
pub async fn get_staged_update(app: tauri::AppHandle) -> Result<Option<update::PendingUpdate>> {
    Ok(update::read_pending(&app))
}

#[command]
pub async fn discard_staged_update(app: tauri::AppHandle) -> Result<()> {
    update::clear_staging(&app);
    Ok(())
}

/// 前端检查结果统一形状（UpToDate 时也返回 manifest 字段的空壳，便于渲染）
#[allow(dead_code)]
fn status_to_value(s: UpdateStatus) -> Result<serde_json::Value> {
    serde_json::to_value(s).map_err(|e| crate::util::error::Error::Other(e.to_string()))
}
