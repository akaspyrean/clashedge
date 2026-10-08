// src-tauri/src/commands/update.rs
//! Portable Updater 命令：检查 / 下载暂存 / 查询暂存状态 / 清除暂存 / 重启安装
//!
//! 应用侧只负责「验签清单 → 取到已验证的更新包并暂存」；最终替换由根启动器
//! 在下次启动时执行（Windows 无法覆盖运行中的自身映像）。
//!
//! `download_update` 无参数——只使用 `check_update`
//! 刚通过 minisign 验签并缓存的后端 manifest。WebView 传入的
//! version/url/hash 一律不参与下载决策，无法被用来构造任意下载参数。

use crate::update::{self, UpdateStatus, VerifiedUpdate};
use crate::util::error::{Error, Result};
use std::time::{Duration, Instant};
use tauri::{command, Manager};

/// 已验签更新清单缓存的有效期：超过后 `download_update` 要求重新 check_update，
/// 避免用陈旧清单下载已被撤回/替换的版本（进程内 TTL，无需持久化）。
const VERIFIED_UPDATE_TTL: Duration = Duration::from_secs(30 * 60);

/// 检查更新并缓存验签材料（命令与启动时静默检查共用）。
pub async fn check_and_cache(app: &tauri::AppHandle) -> Result<UpdateStatus> {
    let outcome = update::check_for_update(app).await?;
    if let UpdateStatus::Available { manifest, .. } = &outcome.status {
        let state = app.state::<crate::AppState>();
        *state.verified_update.lock().unwrap() = Some(VerifiedUpdate {
            manifest: manifest.clone(),
            raw: outcome.raw.clone(),
            signature: outcome.signature.clone(),
            at: Instant::now(),
        });
    }
    Ok(outcome.status)
}

#[command]
pub async fn check_update(app: tauri::AppHandle) -> Result<serde_json::Value> {
    let status = check_and_cache(&app).await?;
    serde_json::to_value(status).map_err(|e| Error::Other(e.to_string()))
}

#[command]
pub async fn download_update(app: tauri::AppHandle) -> Result<crate::update::PendingUpdate> {
    // 只信任本会话刚验签过的 manifest；没有或已过 TTL 则要求先执行检查
    let verified = {
        let state = app.state::<crate::AppState>();
        let cached = state.verified_update.lock().unwrap().clone();
        match cached {
            Some(v) if v.at.elapsed() <= VERIFIED_UPDATE_TTL => v,
            Some(_) => {
                return Err(Error::Other(
                    "已验签的更新清单已过期（距检查超过 30 分钟），请重新检查更新".to_string(),
                ))
            }
            None => {
                return Err(Error::Other(
                    "请先检查更新（下载只能使用已验签的更新清单）".to_string(),
                ))
            }
        }
    };
    // 纵深防御：缓存中的 URL 同样过禁段校验（感知 TUN + fake-ip）
    crate::util::fetch::validate_url_app(&app, &verified.manifest.url).await?;
    update::download_and_stage(&app, &verified).await
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

/// 重启并安装已暂存的更新：拉起根启动器（`--wait-pid <本进程>`，等本进程完全退出后
/// 再应用更新），然后正常退出——`RunEvent::Exit` 会先恢复系统代理、再停止 mihomo。
#[command]
pub async fn restart_and_apply_update(app: tauri::AppHandle) -> Result<()> {
    if update::read_pending(&app).is_none() {
        return Err(Error::Other("没有已暂存的更新".to_string()));
    }
    let launcher = crate::util::paths::portable_root()
        .map(|r| r.join("ClashEdge.exe"))
        .filter(|p| p.exists())
        .ok_or_else(|| {
            Error::Other("找不到根启动器 ClashEdge.exe，无法自动安装更新".to_string())
        })?;
    let me = std::env::current_exe().ok();
    if me
        .as_deref()
        .is_some_and(|m| crate::util::process::same_path(m, &launcher))
    {
        return Err(Error::Other(
            "当前不是启动器布局，无法自动安装更新".to_string(),
        ));
    }
    let mut cmd = std::process::Command::new(&launcher);
    cmd.arg("--wait-pid").arg(std::process::id().to_string());
    if let Some(dir) = launcher.parent() {
        cmd.current_dir(dir);
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0000_0008 | 0x0800_0000); // DETACHED_PROCESS | CREATE_NO_WINDOW
    }
    cmd.spawn()
        .map_err(|e| Error::Other(format!("无法启动更新安装程序：{}", e)))?;
    app.exit(0);
    Ok(())
}
