// src-tauri/src/geodata/updater.rs
//! GeoData 手动更新（从 Mihomo releases 下载 geoip.dat / geosite.dat）
//!
//! 本模块提供 GeoIP / GeoSite 数据文件的手动更新能力：
//! - 从配置的 URL 下载数据文件（支持多 URL 兜底）
//! - 采用原子替换（先写临时文件，再重命名）
//! - 失败时回滚到上一版本（重命名 backup 回来）
//! - UI 忙碌状态由调用方管理
//!
//! 路径遵循便携包约定：通过 `crate::util::paths::get_app_data_dir` 获取，
//! 便携模式下自动解析 `<exe_dir>/Data`，否则 Tauri 默认 app_data_dir。

use anyhow::{Context, Result};
use std::path::Path;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tracing::{info, warn};

use crate::geodata::sources::GeoSources;
use tauri::{Emitter, Manager};

/// 单个 geo 数据文件下载上限（200 MB）：geoip.dat / geosite.dat 正常体积
/// 在 10~100 MB 量级，超限视为恶意/损坏源，防止把磁盘写爆。
const MAX_GEO_DATA_BYTES: u64 = 200 * 1024 * 1024;

/// 更新完成后通知前端刷新 geo 状态（托盘触发与命令触发都受益）。
/// 成功/失败都发，前端据此更新「GeoIP/GeoSite 大小」等展示。
pub async fn update_geodata(app_handle: &tauri::AppHandle) -> Result<()> {
    let result = update_geodata_inner(app_handle).await;
    let (ok, msg) = match &result {
        Ok(()) => (true, "ok".to_string()),
        Err(e) => (false, e.to_string()),
    };
    let _ = app_handle.emit(
        "geodata-updated",
        serde_json::json!({ "ok": ok, "error": msg }),
    );
    result
}

async fn update_geodata_inner(app_handle: &tauri::AppHandle) -> Result<()> {
    // 下载源来自应用配置（高级设置里的自定义 URL 优先，默认 URL 兜底），
    // 而不是写死的默认源——否则 set_geodata_urls 存的 URL 永远不生效。
    let sources = geodata_sources(app_handle);

    // GeoIP 与 GeoSite 文件路径（path 助手会创建 geodata 目录）
    let geoip_path = crate::util::paths::get_geoip_path(app_handle)?;
    let geosite_path = crate::util::paths::get_geosite_path(app_handle)?;

    // 更新前先将现有文件备份（{path}.backup）
    let geoip_backup = backup_path(&geoip_path);
    let geosite_backup = backup_path(&geosite_path);

    let files_to_update = vec![
        ("geoip", &geoip_path, &sources.geoip, &geoip_backup),
        ("geosite", &geosite_path, &sources.geosite, &geosite_backup),
    ];

    for (name, final_path, urls, backup_path) in &files_to_update {
        info!("Updating {}...", name);

        // 跳过未配置 URL 的数据类型
        if urls.is_empty() {
            warn!("No URLs configured for {} update", name);
            continue;
        }

        // 先下载到临时文件，逐个 URL 尝试。
        // 事务性：下载失败时清理临时文件再中止，不留半截 .download 文件。
        let temp_path = final_path.with_extension("download");
        let downloaded = match download_file(app_handle, urls, &temp_path).await {
            Ok(n) => n,
            Err(e) => {
                let _ = fs::remove_file(&temp_path).await;
                return Err(anyhow::anyhow!("Failed to download {}: {}", name, e));
            }
        };

        info!("Downloaded {} ({} bytes)", name, downloaded);

        // 原子替换：先备份现有文件，再重命名临时文件为最终文件
        // Step 1: 将现有文件重命名为备份（如果存在）
        if final_path.exists() {
            fs::rename(final_path, backup_path)
                .await
                .context("Failed to backup existing geo data file")?;
            info!("Backed up existing {} to {}", name, backup_path.display());
        }

        // Step 2: 将临时文件重命名为最终文件（同文件系统内原子操作）
        fs::rename(&temp_path, final_path)
            .await
            .context("Failed to replace geo data file atomically")?;
        info!(
            "Atomically replaced {} with {}",
            final_path.display(),
            temp_path.display()
        );

        // Step 3: 替换成功后清理备份
        let _ = fs::remove_file(backup_path).await;
    }

    Ok(())
}

/// 生成备份路径：`geoip.dat` -> `geoip.backup`
fn backup_path(path: &Path) -> std::path::PathBuf {
    path.with_extension("backup")
}

/// 从 URL 列表下载文件，逐个尝试直到成功。
/// 每个 URL 走共享拉取助手 `util::fetch::get_direct_first`：
/// 直连优先，直连不通自动切应用自身代理兜底（软件代理模式不变）。
async fn download_file(
    app_handle: &tauri::AppHandle,
    urls: &[String],
    target_path: &std::path::Path,
) -> Result<usize> {
    for (index, url) in urls.iter().enumerate() {
        info!(
            "Attempting to download from URL {} (attempt {})",
            url,
            index + 1
        );

        // C2 SSRF 防护：目标 URL 必须通过禁段校验（get_direct_first 内部也会再校验，
        // 这里显式校验以给出清晰的逐 URL 拒绝提示）
        if let Err(e) = crate::util::fetch::validate_url(url).await {
            warn!("URL {} rejected: {}", url, e);
            continue;
        }

        match crate::util::fetch::get_direct_first(app_handle, url).await {
            Ok(mut response) => {
                if response.status().is_success() {
                    let mut file = match fs::File::create(target_path).await {
                        Ok(f) => f,
                        Err(e) => {
                            warn!(
                                "Failed to create temp file {}: {}",
                                target_path.display(),
                                e
                            );
                            continue;
                        }
                    };

                    // 流式写入响应到文件；累计字节数超上限即中止并清理临时文件（C8）
                    let mut total: u64 = 0;
                    loop {
                        match response.chunk().await {
                            Ok(Some(chunk)) => {
                                total += chunk.len() as u64;
                                if total > MAX_GEO_DATA_BYTES {
                                    let _ = file.flush().await;
                                    drop(file);
                                    let _ = fs::remove_file(target_path).await;
                                    return Err(anyhow::anyhow!(
                                        "Download exceeds {} MB limit, aborted",
                                        MAX_GEO_DATA_BYTES / 1024 / 1024
                                    ));
                                }
                                match file.write(&chunk).await {
                                    Ok(_) => {}
                                    Err(e) => {
                                        warn!(
                                            "Failed to write chunk to {}: {}",
                                            target_path.display(),
                                            e
                                        );
                                        let _ = file.flush().await;
                                        return Err(anyhow::anyhow!(
                                            "Failed to write to {}",
                                            target_path.display()
                                        ));
                                    }
                                }
                            }
                            Ok(None) => break,
                            Err(e) => {
                                warn!("Failed to read response from {}: {}", url, e);
                                let _ = file.flush().await;
                                return Err(anyhow::anyhow!(
                                    "Failed to read response body from {}",
                                    url
                                ));
                            }
                        }
                    }

                    // 关闭文件
                    drop(file);

                    // 校验下载文件大小
                    let size = match fs::metadata(target_path).await {
                        Ok(metadata) => metadata.len() as usize,
                        Err(_) => 0,
                    };

                    if size > 0 {
                        info!(
                            "Downloaded {} successfully ({} bytes via {})",
                            target_path.display(),
                            size,
                            url
                        );
                        return Ok(size);
                    } else {
                        warn!("Downloaded file is empty: {}", target_path.display());
                        // 删除空的临时文件
                        let _ = fs::remove_file(target_path).await;
                        continue; // 尝试下一个 URL
                    }
                } else {
                    warn!("URL returned status: {}", response.status());
                }
            }
            Err(e) => {
                warn!("Download failed from {}: {}", url, e);
                continue; // 尝试下一个 URL
            }
        }
    }

    Err(anyhow::anyhow!(
        "All {} URLs failed to download",
        urls.len()
    ))
}

/// Roll back GeoIP/GeoSite files to the previous version
///
/// 如果更新失败，此函数从备份（.backup）恢复之前的 GeoIP / GeoSite 文件。
pub async fn rollback_geodata(app_handle: &tauri::AppHandle) -> Result<()> {
    let result = rollback_geodata_inner(app_handle).await;
    let (ok, msg) = match &result {
        Ok(()) => (true, "ok".to_string()),
        Err(e) => (false, e.to_string()),
    };
    let _ = app_handle.emit(
        "geodata-rolled-back",
        serde_json::json!({ "ok": ok, "error": msg }),
    );
    result
}

async fn rollback_geodata_inner(app_handle: &tauri::AppHandle) -> Result<()> {
    let geoip_path = crate::util::paths::get_geoip_path(app_handle)?;
    let geosite_path = crate::util::paths::get_geosite_path(app_handle)?;
    let geoip_backup = backup_path(&geoip_path);
    let geosite_backup = backup_path(&geosite_path);

    let mut rolled_back = false;

    // 尝试恢复 GeoIP
    if geoip_path.exists() && geoip_backup.exists() {
        let _ = fs::remove_file(&geoip_path).await;
        fs::rename(&geoip_backup, &geoip_path)
            .await
            .context("Failed to rollback geoip.dat")?;
        info!("Rolled back geoip.dat");
        rolled_back = true;
    }

    // 尝试恢复 GeoSite
    if geosite_path.exists() && geosite_backup.exists() {
        let _ = fs::remove_file(&geosite_path).await;
        fs::rename(&geosite_backup, &geosite_path)
            .await
            .context("Failed to rollback geosite.dat")?;
        info!("Rolled back geosite.dat");
        rolled_back = true;
    }

    if !rolled_back {
        warn!("No geo data files were rolled back (none were updated or backed up)");
    }

    Ok(())
}

/// 查询 GeoIP / GeoSite 文件的当前状态
///
/// 返回 JSON 形状：
/// ```json
/// {
///   "geoip":  { "exists": true, "size": 12345, "url": "https://..." },
///   "geosite": { "exists": false, "size": 0, "url": "https://..." }
/// }
/// ```
pub async fn get_status(app_handle: &tauri::AppHandle) -> serde_json::Value {
    let sources = geodata_sources(app_handle);

    let geoip_path = crate::util::paths::get_geoip_path(app_handle);
    let geosite_path = crate::util::paths::get_geosite_path(app_handle);

    let geoip = file_status(geoip_path.ok(), sources.geoip_primary());
    let geosite = file_status(geosite_path.ok(), sources.geosite_primary());

    serde_json::json!({
        "geoip": geoip,
        "geosite": geosite,
    })
}

/// 构建单个数据文件的状态 JSON
fn file_status(path: Option<std::path::PathBuf>, primary_url: Option<&str>) -> serde_json::Value {
    let (exists, size) = match path {
        Some(p) => match std::fs::metadata(&p) {
            Ok(metadata) => (true, metadata.len()),
            Err(_) => (false, 0),
        },
        None => (false, 0),
    };

    serde_json::json!({
        "exists": exists,
        "size": size,
        "url": primary_url.unwrap_or(""),
    })
}

/// 从应用配置构建实际生效的下载源：
/// 高级设置里的自定义 URL（advanced.geoip-url / geosite-url）作为首选，
/// 默认 MetaCubeX URL 列表作为兜底（自定义失败时仍能下载成功）。
fn geodata_sources(app_handle: &tauri::AppHandle) -> GeoSources {
    let defaults = GeoSources::new();
    let advanced = &app_handle
        .state::<crate::AppState>()
        .config_manager
        .lock()
        .unwrap()
        .get_config()
        .advanced;
    GeoSources {
        geoip: prepend_urls(defaults.geoip, &advanced.geoip_url),
        geosite: prepend_urls(defaults.geosite, &advanced.geosite_url),
        geox: defaults.geox,
    }
}

/// 把用户自定义 URL 放到列表首位（非空才加入），再补默认列表（去重兜底）。
fn prepend_urls(defaults: Vec<String>, custom: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let trimmed = custom.trim();
    if !trimmed.is_empty() {
        out.push(trimmed.to_string());
    }
    for url in defaults {
        if !out.contains(&url) {
            out.push(url);
        }
    }
    out
}
