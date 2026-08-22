// src-tauri/src/update/mod.rs
//! Portable Updater（0.8.10，AUDIT Phase 3 重做）
//!
//! 半成品 Tauri updater 已移除；便携包更新走自有链路：
//!
//! ```text
//! 检查 portable-manifest.json → 版本比较 → 流式下载 versioned ZIP
//!   → SHA256 校验 → 暂存 Data/update-staging/ + 写 pending.json
//!   → 下次由根启动器在拉起内层前应用（解压→结构校验→替换 App/→保留 Data/）
//!   → 失败回滚（启动器负责：App.new 校验不过就保留旧 App/）
//! ```
//!
//! 职责边界：
//! - 本模块只做「检查 / 下载 / 验签 / 暂存」，绝不在运行中自我替换；
//!   Windows 无法覆盖运行中的自身映像，最终交换由启动器在下次启动完成；
//! - ZIP 解压与 App/ 替换在 tools/ClashEdge.Launcher.R8.2.cs 中实现，
//!   启动器是唯一有权限安全替换内层映像的组件。

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::{info, warn};

use crate::util::error::{Error, Result};

/// 更新清单地址（GitHub Releases latest 钉定产物）
pub const UPDATE_ENDPOINT: &str =
    "https://github.com/akaspyrean/clashedge/releases/latest/download/portable-manifest.json";

/// 单次下载大小上限（便携包 ZIP 正常 <100 MB）
const MAX_UPDATE_BYTES: u64 = 300 * 1024 * 1024;
/// 整体下载 deadline
const DOWNLOAD_DEADLINE: std::time::Duration = std::time::Duration::from_secs(900);

/// 更新清单（tools/make_update_manifest.py 生成的 portable-manifest.json）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateManifest {
    pub version: String,
    pub url: String,
    /// ZIP 的小写十六进制 SHA256
    pub sha256: String,
    #[serde(default)]
    pub notes: String,
}

/// 检查结果
#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum UpdateStatus {
    UpToDate {
        current: String,
    },
    Available {
        current: String,
        #[serde(flatten)]
        manifest: UpdateManifest,
    },
}

/// 暂存区 pending 记录（启动器据此应用更新）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingUpdate {
    pub version: String,
    /// 暂存 ZIP 的绝对路径
    pub zip_path: String,
    /// 已验证的 SHA256（启动器应用前复验）
    pub sha256: String,
}

/// 当前应用版本（Cargo 包版本）
pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// 解析 "x.y.z" 为 (x, y, z)；解析失败返回 None
fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let mut it = v.trim().trim_start_matches('v').split('.');
    let a = it.next()?.parse().ok()?;
    let b = it.next()?.parse().ok()?;
    let c = it.next().unwrap_or("0").parse().ok()?;
    Some((a, b, c))
}

/// 远端版本是否比当前新
fn is_newer(remote: &str, current: &str) -> bool {
    match (parse_version(remote), parse_version(current)) {
        (Some(r), Some(c)) => r > c,
        _ => false,
    }
}

/// 检查更新：拉取 manifest 并比较版本。
/// 网络失败/清单非法一律返回 Err——不假装"已是最新"。
pub async fn check_for_update(app: &tauri::AppHandle) -> Result<UpdateStatus> {
    let resp = crate::util::fetch::get_direct_first(app, UPDATE_ENDPOINT).await?;
    if !resp.status().is_success() {
        return Err(Error::Other(format!(
            "update manifest fetch failed: HTTP {}",
            resp.status()
        )));
    }
    let manifest: UpdateManifest = resp
        .json()
        .await
        .map_err(|e| Error::Other(format!("invalid update manifest: {}", e)))?;
    if manifest.url.is_empty() || manifest.sha256.len() != 64 {
        return Err(Error::Other(
            "update manifest missing url or sha256".to_string(),
        ));
    }
    // SSRF：manifest 的 url 也必须过禁段校验（下载时 get_direct_first 还会再验）
    crate::util::fetch::validate_url(&manifest.url).await?;

    let current = current_version().to_string();
    if is_newer(&manifest.version, &current) {
        Ok(UpdateStatus::Available { current, manifest })
    } else {
        Ok(UpdateStatus::UpToDate { current })
    }
}

/// 暂存目录：Data/update-staging/
fn staging_dir(app: &tauri::AppHandle) -> Result<std::path::PathBuf> {
    let dir = crate::util::paths::get_app_data_dir(app)?.join("update-staging");
    std::fs::create_dir_all(&dir)
        .map_err(|e| Error::Io(std::io::Error::new(e.kind(), e.to_string())))?;
    Ok(dir)
}

/// 读取暂存中的 pending 更新（无/损坏 → None）
pub fn read_pending(app: &tauri::AppHandle) -> Option<PendingUpdate> {
    let path = staging_dir(app).ok()?.join("pending.json");
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

/// 清除暂存（用户取消或启动器应用成功后调用）
pub fn clear_staging(app: &tauri::AppHandle) {
    if let Ok(dir) = staging_dir(app) {
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// 下载并暂存更新：流式写入 → 同步计算 SHA256 → 与 manifest 比对
/// → 写 pending.json。任何一步失败都清理暂存并返回 Err。
pub async fn download_and_stage(
    app: &tauri::AppHandle,
    manifest: &UpdateManifest,
) -> Result<PendingUpdate> {
    let dir = staging_dir(app)?;
    let zip_path = dir.join(format!("ClashEdge-{}.zip", manifest.version));
    let tmp_path = dir.join("update.zip.download");

    // 清掉上次残留的半截下载
    let _ = std::fs::remove_file(&tmp_path);

    let expected = manifest.sha256.to_lowercase();
    let download_result = {
        let tmp_path = tmp_path.clone();
        let url = manifest.url.clone();
        tokio::time::timeout(DOWNLOAD_DEADLINE, async move {
            let mut resp = crate::util::fetch::get_direct_first_streaming(app, &url).await?;
            if !resp.status().is_success() {
                return Err(Error::Other(format!(
                    "download failed: HTTP {}",
                    resp.status()
                )));
            }
            if let Some(len) = resp.content_length() {
                if len > MAX_UPDATE_BYTES {
                    return Err(Error::Other(format!(
                        "update package too large: {} bytes",
                        len
                    )));
                }
            }
            use tokio::io::AsyncWriteExt;
            let mut file = tokio::fs::File::create(&tmp_path).await?;
            let mut hasher = Sha256::new();
            let mut total: u64 = 0;
            loop {
                match resp.chunk().await {
                    Ok(Some(chunk)) => {
                        total += chunk.len() as u64;
                        if total > MAX_UPDATE_BYTES {
                            let _ = file.flush().await;
                            return Err(Error::Other(format!(
                                "update download exceeds {} MB limit",
                                MAX_UPDATE_BYTES / 1024 / 1024
                            )));
                        }
                        hasher.update(&chunk);
                        file.write_all(&chunk)
                            .await
                            .map_err(|e| Error::Io(std::io::Error::new(e.kind(), e.to_string())))?;
                    }
                    Ok(None) => break,
                    Err(e) => return Err(Error::Other(format!("download interrupted: {}", e))),
                }
            }
            file.flush().await.ok();
            Ok(hex_encode(&hasher.finalize()))
        })
        .await
    };
    let downloaded_sha = match download_result {
        Ok(inner) => inner?,
        Err(_) => {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(Error::Other("update download timed out".to_string()));
        }
    };

    if downloaded_sha != expected {
        let _ = std::fs::remove_file(&tmp_path);
        warn!(
            "update sha256 mismatch: expected {}, got {}",
            expected, downloaded_sha
        );
        return Err(Error::Other(
            "SHA256 mismatch: update package rejected".to_string(),
        ));
    }

    // 原子就位：rename 到正式名
    std::fs::rename(&tmp_path, &zip_path).map_err(|e| {
        Error::Io(std::io::Error::new(
            e.kind(),
            format!("rename staged zip: {}", e),
        ))
    })?;

    let pending = PendingUpdate {
        version: manifest.version.clone(),
        zip_path: zip_path.to_string_lossy().to_string(),
        sha256: expected,
    };
    let pending_json =
        serde_json::to_string_pretty(&pending).map_err(|e| Error::Other(e.to_string()))?;
    crate::util::atomic::atomic_write(&dir.join("pending.json"), pending_json.as_bytes())?;
    info!(
        "Update {} staged and verified (sha256 ok); launcher will apply on next start",
        pending.version
    );
    Ok(pending)
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_compare() {
        assert!(is_newer("0.8.10", "0.8.9"));
        assert!(is_newer("0.9.0", "0.8.99"));
        assert!(!is_newer("0.8.9", "0.8.9"));
        assert!(!is_newer("0.8.8", "0.8.9"));
        assert!(!is_newer("garbage", "0.8.9"));
    }

    #[test]
    fn parses_v_prefixed_and_partial() {
        assert_eq!(parse_version("v1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("1.2"), Some((1, 2, 0)));
        assert_eq!(parse_version(""), None);
    }

    #[test]
    fn hex_encode_works() {
        assert_eq!(hex_encode(&[0xde, 0xad]), "dead");
    }
}
