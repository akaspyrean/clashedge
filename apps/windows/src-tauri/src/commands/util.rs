// src-tauri/src/commands/util.rs
//! 工具命令：打开目录、版本、语言等

use crate::util::error::Result;
use tauri::{command, Emitter, Manager};

#[command]
pub async fn open_data_dir(app: tauri::AppHandle) -> Result<()> {
    let data_dir = crate::util::paths::get_app_data_dir(&app)?;
    crate::util::paths::open_in_explorer(&data_dir)
}

#[command]
pub async fn open_logs_dir(app: tauri::AppHandle) -> Result<()> {
    let logs_dir = crate::util::paths::get_logs_dir(&app)?;
    crate::util::paths::open_in_explorer(&logs_dir)
}

#[command]
pub fn get_app_version() -> String {
    crate::update::current_version().to_string()
}

/// 是否由自启动拉起（命令行带 --clash-edge-autostart）。
/// 自启动时应只驻留托盘不显示窗口，手动启动才显示主窗口。
#[command]
pub fn is_autostart() -> bool {
    std::env::args().any(|a| a == "--clash-edge-autostart")
}

/// 当前是否已开启开机自启（注册表 Run 键 + StartupApproved）。
#[command]
pub fn get_autostart() -> Result<bool> {
    crate::util::autostart::get_autostart()
}

/// 设置开机自启。启用时注册表 Run 键指向根启动器 `--clash-edge-autostart`，
/// 保证开机静默启动。完成后刷新托盘菜单（开机自启项勾选态跟随）。
#[command]
pub async fn set_autostart(app: tauri::AppHandle, enable: bool) -> Result<()> {
    crate::util::autostart::set_autostart(enable)?;
    crate::core::runtime::refresh_tray(&app).await?;
    let _ = app.emit("autostart-changed", serde_json::json!({ "enable": enable }));
    Ok(())
}

#[command]
pub fn get_supported_locales() -> Vec<String> {
    crate::i18n::loader::supported_locales()
        .iter()
        .map(|s| s.to_string())
        .collect()
}

#[command]
pub async fn set_locale(app: tauri::AppHandle, locale: String) -> Result<()> {
    // 经 AppController：事务锁 + 降级守卫 + 持久化 + 托盘刷新
    let state = app.state::<crate::AppState>();
    state.controller.set_locale(&app, locale).await
}

#[command]
pub fn get_i18n_messages(locale: String) -> Result<serde_json::Value> {
    let table = crate::i18n::loader::messages_for_locale(&locale);
    Ok(serde_json::to_value(table)?)
}
