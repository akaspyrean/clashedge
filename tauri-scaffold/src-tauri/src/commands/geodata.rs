// src-tauri/src/commands/geodata.rs
//! 地理数据命令：手动更新、状态查询、URL 配置

use crate::util::error::Result;
use tauri::{command, State};

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
        "geox_url": advanced.geox_url,
        "geoip_url": advanced.geoip_url,
        "geosite_url": advanced.geosite_url,
    }))
}

#[command]
pub async fn set_geodata_urls(
    state: State<'_, crate::AppState>,
    urls: serde_json::Value,
) -> Result<()> {
    let mut config_guard = state.config_manager.lock().unwrap();
    let mut config = config_guard.get_config().clone();
    if let Some(geox) = urls.get("geox_url").and_then(|v| v.as_str()) {
        config.advanced.geox_url = geox.to_string();
    }
    if let Some(geoip) = urls.get("geoip_url").and_then(|v| v.as_str()) {
        config.advanced.geoip_url = geoip.to_string();
    }
    if let Some(geosite) = urls.get("geosite_url").and_then(|v| v.as_str()) {
        config.advanced.geosite_url = geosite.to_string();
    }
    config_guard.set_config(config)?;
    Ok(())
}
