// src-tauri/src/geodata/updater.rs
//! GeoData 更新（GeoIP.dat / GeoSite.dat）与"更新规则 + GeoData"总入口
//!
//! 重要修正（审计 B3）：mihomo 以 `-d <Data>` 启动，只会读取 **Data 根目录**下的
//! `GeoIP.dat` / `GeoSite.dat`。旧实现把文件写到 `Data/geodata/geoip.dat`——mihomo
//! 永远读不到，更新实际上是空操作。现在直接写入 Data 根目录，替换成功后强制重载核心。
//!
//! 流程：
//! 1. 先尝试签名规则清单（`geodata::rules`，同时覆盖规则集与清单中列出的 geodata）；
//!    清单不可用只记 warn，不阻断后续；
//! 2. 逐个 URL 下载 GeoIP / GeoSite（自定义源优先，内置 MetaCubeX 兜底；任何一个 URL
//!    无论连接失败还是**响应体中途读取失败**，都会继续尝试下一个镜像）；
//! 3. 全部下载并通过结构校验后，两个文件**事务式一起替换**，任一步失败整体回滚；
//! 4. 替换成功强制重载核心使新数据生效。

use std::path::{Path, PathBuf};
use std::time::Duration;

use tracing::{info, warn};

use crate::geodata::download::{download_to_file, replace_all};
use crate::geodata::sources::GeoSources;
use crate::util::error::{Error, Result};
use tauri::{Emitter, Manager};

/// 单个 geo 数据文件下载上限（200 MB）
const MAX_GEO_DATA_BYTES: u64 = 200 * 1024 * 1024;
/// 单个数据文件的整体下载 deadline
const DOWNLOAD_DEADLINE: Duration = Duration::from_secs(600);
/// 自动更新的最小间隔
const AUTO_UPDATE_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

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

async fn update_geodata_inner(app: &tauri::AppHandle) -> Result<()> {
    // 1. 签名规则清单（尽力而为：清单未发布 / 网络不通不阻断 geodata 更新）
    match crate::geodata::rules::update_rules(app).await {
        Ok(o) => info!(
            "Signed rules update ok (v{}, updated {} files, up_to_date={})",
            o.version,
            o.updated.len(),
            o.up_to_date
        ),
        Err(e) => warn!("Signed rules update skipped: {}", e),
    }
    // 2. GeoData 文件
    update_geo_files(app).await?;
    stamp_updated(app);
    Ok(())
}

/// 下载 GeoIP / GeoSite 并事务式替换。
async fn update_geo_files(app: &tauri::AppHandle) -> Result<()> {
    let sources = geodata_sources(app);
    let targets: Vec<(&str, PathBuf, Vec<String>)> = vec![
        (
            "geoip",
            crate::util::paths::get_geoip_path(app)?,
            sources.geoip,
        ),
        (
            "geosite",
            crate::util::paths::get_geosite_path(app)?,
            sources.geosite,
        ),
    ];

    let mut staged: Vec<(PathBuf, PathBuf)> = Vec::new();
    let cleanup = |staged: &[(PathBuf, PathBuf)]| {
        for (tmp, _) in staged {
            let _ = std::fs::remove_file(tmp);
        }
    };

    for (name, final_path, urls) in &targets {
        if urls.is_empty() {
            warn!("No URLs configured for {} update", name);
            continue;
        }
        let tmp = {
            let mut n = final_path.file_name().unwrap_or_default().to_os_string();
            n.push(".download");
            final_path.with_file_name(n)
        };
        match download_first_valid(app, name, urls, &tmp).await {
            Ok(size) => {
                info!("Downloaded {} ({} bytes)", name, size);
                staged.push((tmp, final_path.clone()));
            }
            Err(e) => {
                cleanup(&staged);
                return Err(Error::Other(format!("Failed to update {}: {}", name, e)));
            }
        }
    }
    if staged.is_empty() {
        return Ok(());
    }

    // 3. 事务式一起替换
    if let Err(e) = replace_all(&staged) {
        cleanup(&staged);
        return Err(e);
    }
    // 4. 让核心读取新数据
    if let Err(e) = crate::geodata::rules::reload_core(app).await {
        warn!("GeoData replaced but core reload failed: {}", e);
    }
    Ok(())
}

/// 依次尝试每个 URL，直到得到一个通过结构校验的文件（落在 `tmp`）。
/// 连接失败、HTTP 非 2xx、**响应体中途失败**、校验失败都会换下一个镜像。
async fn download_first_valid(
    app: &tauri::AppHandle,
    name: &str,
    urls: &[String],
    tmp: &Path,
) -> Result<u64> {
    let mut last: Option<String> = None;
    for (i, url) in urls.iter().enumerate() {
        let shown = crate::util::fetch::redact_url_for_log(url);
        info!(
            "Attempting {} from URL {} ({}/{})",
            name,
            shown,
            i + 1,
            urls.len()
        );
        if let Err(e) = crate::util::fetch::validate_url_app(app, url).await {
            warn!("URL {} rejected: {}", shown, e);
            last = Some(e.to_string());
            continue;
        }
        match download_to_file(app, url, tmp, MAX_GEO_DATA_BYTES, DOWNLOAD_DEADLINE).await {
            Ok((size, _sha)) => match validate_geodata_file(tmp, name) {
                Ok(()) => return Ok(size),
                Err(e) => {
                    warn!(
                        "Content validation failed for {} via {}: {}",
                        name, shown, e
                    );
                    let _ = std::fs::remove_file(tmp);
                    last = Some(e.to_string());
                }
            },
            Err(e) => {
                warn!("Download of {} failed via {}: {}", name, shown, e);
                last = Some(e.to_string());
            }
        }
    }
    Err(Error::Other(format!(
        "all {} URLs failed{}",
        urls.len(),
        last.map(|l| format!(" (last error: {})", l))
            .unwrap_or_default()
    )))
}

/// GeoData 结构校验：拒绝 HTML 错误页、过小的损坏文件；`.dat` 必须以 protobuf
/// 字段 1 的 length-delimited 标签（0x0A）开头；`.mmdb` 必须含 MaxMind 元数据标记。
pub fn validate_geodata_file(path: &Path, name: &str) -> Result<()> {
    use std::io::{Read, Seek, SeekFrom};
    const MIN_GEODATA_SIZE: u64 = 100_000;
    let mut file = std::fs::File::open(path)?;
    let size = file.metadata()?.len();
    if size < MIN_GEODATA_SIZE {
        return Err(Error::Other(format!(
            "{} too small ({} bytes, minimum {}); likely an error page",
            name, size, MIN_GEODATA_SIZE
        )));
    }
    let mut head = vec![0u8; 1024];
    let n = file.read(&mut head)?;
    head.truncate(n);
    let head_lower = String::from_utf8_lossy(&head).to_ascii_lowercase();
    for marker in ["<html", "<!doctype", "<head", "<title"] {
        if head_lower.contains(marker) {
            return Err(Error::Other(format!(
                "{} looks like HTML (found '{}'); rejecting",
                name, marker
            )));
        }
    }
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".mmdb") {
        // MaxMind DB 的元数据位于文件末尾，以 "\xAB\xCD\xEFMaxMind.com" 标记
        let tail_len = size.min(128 * 1024);
        file.seek(SeekFrom::Start(size - tail_len))?;
        let mut tail = vec![0u8; tail_len as usize];
        file.read_exact(&mut tail)?;
        let marker = b"MaxMind.com";
        if !tail.windows(marker.len()).any(|w| w == marker) {
            return Err(Error::Other(format!(
                "{} is missing MaxMind metadata",
                name
            )));
        }
    } else if (lower.ends_with(".dat") || lower == "geoip" || lower == "geosite")
        && head.first() != Some(&0x0A)
    {
        return Err(Error::Other(format!(
            "{} does not look like a v2ray geodata protobuf file",
            name
        )));
    }
    Ok(())
}

fn stamp_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    crate::util::paths::get_app_data_dir(app)
        .ok()
        .map(|d| d.join(".geo-last-update"))
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn stamp_updated(app: &tauri::AppHandle) {
    if let Some(p) = stamp_path(app) {
        let _ = std::fs::write(p, now_secs().to_string());
    }
}

/// 距上次成功更新是否已超过自动更新间隔（无记录视为已到期）。纯函数便于测试。
pub fn is_due(last: Option<u64>, now: u64, interval: Duration) -> bool {
    match last {
        Some(t) => now.saturating_sub(t) >= interval.as_secs(),
        None => true,
    }
}

/// `geo-auto-update` 开启时由启动流程调用：到期才更新（规则 + GeoData），失败只记日志。
pub async fn auto_update_if_due(app: &tauri::AppHandle) {
    let last = stamp_path(app)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| s.trim().parse::<u64>().ok());
    if !is_due(last, now_secs(), AUTO_UPDATE_INTERVAL) {
        info!("Geo auto-update: not due yet");
        return;
    }
    match update_geodata(app).await {
        Ok(()) => info!("Geo auto-update finished"),
        Err(e) => warn!("Geo auto-update failed: {}", e),
    }
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
fn file_status(path: Option<PathBuf>, primary_url: Option<&str>) -> serde_json::Value {
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
    let advanced = app_handle
        .state::<crate::AppState>()
        .config_manager
        .lock()
        .unwrap()
        .get_config()
        .advanced;
    GeoSources {
        geoip: prepend_urls(defaults.geoip, &advanced.geoip_url),
        geosite: prepend_urls(defaults.geosite, &advanced.geosite_url),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("clashedge-geo-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn dat_validation_accepts_protobuf_and_rejects_html_or_tiny() {
        let d = scratch("dat");
        let mut good = vec![0x0A, 0x05, b'a', b'b', b'c', b'd', b'e'];
        good.resize(200_000, 0);
        std::fs::write(d.join("GeoIP.dat"), &good).unwrap();
        assert!(validate_geodata_file(&d.join("GeoIP.dat"), "geoip").is_ok());

        std::fs::write(d.join("tiny.dat"), [0x0A, 1, 2]).unwrap();
        assert!(validate_geodata_file(&d.join("tiny.dat"), "geoip").is_err());

        let mut html = b"<!DOCTYPE html><html>".to_vec();
        html.resize(200_000, b' ');
        std::fs::write(d.join("page.dat"), &html).unwrap();
        assert!(validate_geodata_file(&d.join("page.dat"), "geoip").is_err());

        let wrong = vec![0xFF; 200_000];
        std::fs::write(d.join("wrong.dat"), &wrong).unwrap();
        assert!(validate_geodata_file(&d.join("wrong.dat"), "geosite").is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn mmdb_validation_requires_maxmind_marker() {
        let d = scratch("mmdb");
        let mut data = vec![0u8; 150_000];
        data.extend_from_slice(b"\xAB\xCD\xEFMaxMind.com");
        data.extend_from_slice(&[0u8; 64]);
        std::fs::write(d.join("Country.mmdb"), &data).unwrap();
        assert!(validate_geodata_file(&d.join("Country.mmdb"), "Country.mmdb").is_ok());
        std::fs::write(d.join("bad.mmdb"), vec![0u8; 150_000]).unwrap();
        assert!(validate_geodata_file(&d.join("bad.mmdb"), "bad.mmdb").is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn auto_update_due_logic() {
        let day = Duration::from_secs(86_400);
        assert!(is_due(None, 1_000_000, day));
        assert!(!is_due(Some(1_000_000), 1_000_000 + 3_600, day));
        assert!(is_due(Some(1_000_000), 1_000_000 + 86_400, day));
        // 时钟回拨：不会因下溢而误判为到期
        assert!(!is_due(Some(2_000_000), 1_000_000, day));
    }

    #[test]
    fn custom_url_goes_first_without_duplicates() {
        let defaults = vec!["https://a".to_string(), "https://b".to_string()];
        assert_eq!(
            prepend_urls(defaults.clone(), " https://c "),
            vec!["https://c", "https://a", "https://b"]
        );
        assert_eq!(prepend_urls(defaults.clone(), "https://a"), defaults);
        assert_eq!(prepend_urls(defaults.clone(), ""), defaults);
    }
}
