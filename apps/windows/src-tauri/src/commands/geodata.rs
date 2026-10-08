// src-tauri/src/commands/geodata.rs
//! 地理数据命令：手动更新、状态查询、URL 配置

use crate::util::error::Result;
use tauri::{command, Manager, State};

#[command]
pub async fn update_geodata(app: tauri::AppHandle) -> Result<()> {
    crate::geodata::updater::update_geodata(&app).await?;
    Ok(())
}

#[command]
pub async fn get_geodata_status(app: tauri::AppHandle) -> Result<serde_json::Value> {
    Ok(crate::geodata::updater::get_status(&app).await)
}

#[command]
pub async fn get_geodata_urls(state: State<'_, crate::AppState>) -> Result<serde_json::Value> {
    let config_guard = state.config_manager.lock().unwrap();
    let advanced = &config_guard.get_config().advanced;
    Ok(serde_json::json!({
        "geoip_url": advanced.geoip_url,
        "geosite_url": advanced.geosite_url,
    }))
}

#[command]
pub async fn set_geodata_urls(app: tauri::AppHandle, urls: serde_json::Value) -> Result<()> {
    let pick = |k: &str| urls.get(k).and_then(|v| v.as_str()).map(|s| s.to_string());
    // 经 AppController：事务锁 + 降级守卫 + URL 校验
    let state = app.state::<crate::AppState>();
    state
        .controller
        .set_geodata_urls(&app, pick("geoip_url"), pick("geosite_url"))
        .await
}
