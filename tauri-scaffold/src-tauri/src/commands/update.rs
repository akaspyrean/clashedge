// src-tauri/src/commands/update.rs
//! Portable Updater 命令：检查 / 下载暂存 / 查询暂存状态 / 清除暂存
//!
//! 应用侧只负责「验签清单 → 取到已验证的更新包并暂存」；最终替换由根启动器
//! 在下次启动时执行（Windows 无法覆盖运行中的自身映像）。
//!
//! P0-6（Release Gate）：`download_update` 无参数——只使用 `check_update`
//! 刚通过 minisign 验签并缓存的后端 manifest。WebView 传入的
//! version/url/hash 一律不参与下载决策，无法被用来构造任意下载参数。

use crate::update::{self, UpdateStatus};
use crate::util::error::{Error, Result};
use tauri::{command, Manager};

#[command]
pub async fn check_update(app: tauri::AppHandle) -> Result<serde_json::Value> {
    let status = update::check_for_update(&app).await?;
    // 验签成功且确有新版本 → 缓存 manifest，供 download_update 使用
    if let UpdateStatus::Available { manifest, .. } = &status {
        let state = app.state::<crate::AppState>();
        *state.verified_update.lock().unwrap() = Some(manifest.clone());
    }
    serde_json::to_value(status).map_err(|e| crate::util::error::Error::Other(e.to_string()))
}

#[command]
pub async fn download_update(app: tauri::AppHandle) -> Result<crate::update::PendingUpdate> {
    // 只信任本会话刚验签过的 manifest；没有则要求先执行检查
    let manifest = {
        let state = app.state::<crate::AppState>();
        let cached = state.verified_update.lock().unwrap().clone();
        cached.ok_or_else(|| {
            Error::Other("请先检查更新（下载只能使用已验签的更新清单）".to_string())
        })?
    };
    // 纵深防御：缓存中的 URL 同样过禁段校验
    crate::util::fetch::validate_url(&manifest.url).await?;
    update::download_and_stage(&app, &manifest).await
}

#[command]
pub async fn get_staged_update(app: tauri::AppHandle) -> Result<Option<update::PendingUpdate>> {
    Ok(update::read_pending(&app))
}

#[command]
pub async fn discard_staged_update(app: tauri::AppHandle) -> Result<()> {
    // 用户主动放弃：同时清除后端缓存的已验签 manifest，避免下次误用旧清单
    {
        let state = app.state::<crate::AppState>();
        *state.verified_update.lock().unwrap() = None;
    }
    update::clear_staging(&app);
    Ok(())
}
