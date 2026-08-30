// src-tauri/src/update/mod.rs
//! Portable Updater（真实签名信任链）
//!
//! 便携包更新走自有链路：
//!
//! ```text
//! 内置公钥（编译期固定）
//!   ↓
//! 下载 portable-manifest.json + portable-manifest.json.minisig
//!   ↓
//! minisign 验签 manifest（未签名 / 错签名 / 公钥不匹配 → 一律拒绝）
//!   ↓
//! 解析 manifest（version/url/sha256 从此才可信）
//!   ↓
//! 流式下载 ZIP → SHA256 与 manifest 比对
//!   ↓
//! 暂存 Data/update-staging/ + 写 pending.json
//!   ↓
//! 下次由根启动器在拉起内层前应用（见 packaging/windows/launcher/ 下的启动器源码，
//! 启动器带更新事务 journal，断电可恢复；Data/ 永不被替换）
//! ```
//!
//! 安全边界：
//! - 签名不是 optional：公钥未配置或验签失败时更新功能整体不可用；
//! - 前端传入的 version/url/hash 一律不信任——`download_update` 命令无参数，
//!   只使用 `check_update` 刚验证过并缓存的后端 manifest；
//! - 本模块只做「检查 / 验签 / 下载 / 暂存」，绝不在运行中自我替换。

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::{info, warn};

use crate::util::error::{Error, Result};

/// 更新清单地址（GitHub Releases latest 钉定产物）。
/// 客户端同时拉取 `<endpoint>.minisig` 作为其 minisign 签名。
pub const UPDATE_ENDPOINT: &str =
    "https://github.com/akaspyrean/clashedge/releases/latest/download/portable-manifest.json";

/// 更新清单验签公钥（minisign 公钥，base64）。
///
/// 编译期由环境变量 `CLASHEDGE_UPDATE_PUBKEY` 注入（release workflow 从
/// 同一枚私钥对应的仓库 Secret 传入）。为空 = 更新链不可用，check_update
/// 返回明确错误——绝不降级成"纯 SHA256 无身份校验"的自动更新。
pub const UPDATE_PUBLIC_KEY: &str = match option_env!("CLASHEDGE_UPDATE_PUBKEY") {
    Some(k) => k,
    None => "",
};

/// 单次下载大小上限（便携包 ZIP 正常 <100 MB）
const MAX_UPDATE_BYTES: u64 = 300 * 1024 * 1024;
/// 整体下载 deadline
const DOWNLOAD_DEADLINE: std::time::Duration = std::time::Duration::from_secs(900);

/// 更新清单（scripts/release/make-update-manifest.py 生成的 portable-manifest.json）
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

/// 用内置公钥验证 manifest 的 minisign 签名。
///
/// - `manifest_bytes`：manifest 文件的原始字节（验签对象是文件本身）；
/// - `sig_file_text`：`.minisig` 文件全文（第 2 行是 base64 签名 blob）。
///
/// 未签名、格式错误、公钥不匹配、数据被篡改 → 全部 Err。
pub fn verify_manifest_signature(
    pubkey_b64: &str,
    manifest_bytes: &[u8],
    sig_file_text: &str,
) -> Result<()> {
    if pubkey_b64.trim().is_empty() {
        return Err(Error::Other(
            "更新验签公钥未配置（CLASHEDGE_UPDATE_PUBKEY）；拒绝接受未签名清单".to_string(),
        ));
    }
    let pk = minisign_verify::PublicKey::from_base64(pubkey_b64.trim())
        .map_err(|e| Error::Other(format!("内置更新公钥非法：{}", e)))?;
    let signature = minisign_verify::Signature::decode(sig_file_text)
        .map_err(|e| Error::Other(format!("签名解码失败（.minisig 格式非法）：{}", e)))?;
    // 现代 minisign（prehashed，"ED"）直接通过；旧版非预哈希签名（"Ed"）
    // 需要显式允许 legacy 模式。两者都验不过才算不可信。
    match pk.verify(manifest_bytes, &signature, false) {
        Ok(()) => Ok(()),
        Err(minisign_verify::Error::UnexpectedAlgorithm) => pk
            .verify(manifest_bytes, &signature, true)
            .map_err(|_| Error::Other("更新清单签名验证失败：来源不可信，已拒绝".to_string())),
        Err(_) => Err(Error::Other(
            "更新清单签名验证失败：来源不可信，已拒绝".to_string(),
        )),
    }
}

/// 下载 `.minisig` 签名文件文本。缺失（HTTP 失败/非成功码）一律 Err——
/// 未签名清单不可接受。
async fn fetch_signature_text(app: &tauri::AppHandle) -> Result<String> {
    let sig_url = format!("{}.minisig", UPDATE_ENDPOINT);
    let resp = crate::util::fetch::get_direct_first(app, &sig_url).await?;
    if !resp.status().is_success() {
        return Err(Error::Other(format!(
            "update manifest signature fetch failed: HTTP {}（未签名发布不受信任）",
            resp.status()
        )));
    }
    resp.text()
        .await
        .map_err(|e| Error::Other(format!("invalid update manifest signature: {}", e)))
}

/// 检查更新：下载 manifest + 签名 → 验签 → 解析比较版本。
/// 签名无效 / 清单非法 / 网络失败一律返回 Err——不假装"已是最新"。
/// 成功返回的 manifest 已通过信任链，可直接用于下载暂存。
pub async fn check_for_update(app: &tauri::AppHandle) -> Result<UpdateStatus> {
    if UPDATE_PUBLIC_KEY.trim().is_empty() {
        return Err(Error::Other(
            "自动更新不可用：客户端未内置更新公钥（构建配置缺失）".to_string(),
        ));
    }

    let resp = crate::util::fetch::get_direct_first(app, UPDATE_ENDPOINT).await?;
    if !resp.status().is_success() {
        return Err(Error::Other(format!(
            "update manifest fetch failed: HTTP {}",
            resp.status()
        )));
    }
    let manifest_bytes = resp
        .bytes()
        .await
        .map_err(|e| Error::Other(format!("update manifest read failed: {}", e)))?;

    // 先验签，再解析内容——未通过信任链的 manifest 内容一律不看。
    let sig_text = fetch_signature_text(app).await?;
    verify_manifest_signature(UPDATE_PUBLIC_KEY, &manifest_bytes, &sig_text)?;

    let manifest: UpdateManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|e| Error::Other(format!("invalid update manifest: {}", e)))?;
    if manifest.url.is_empty() || manifest.sha256.len() != 64 {
        return Err(Error::Other(
            "update manifest missing url or sha256".to_string(),
        ));
    }
    // SSRF：manifest 的 url 也必须过禁段校验（下载时 get_direct_first 还会再验）
    crate::util::fetch::validate_url(&manifest.url).await?;

    info!(
        "Update manifest signature verified (v{}, sha256 {}...)",
        manifest.version,
        &manifest.sha256[..12]
    );

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

    // ---- 签名信任链 ----

    /// 官方 minisign 测试向量（jedisct1/minisign 与 minisign-verify crate
    /// 同源的公开测试密钥对，仅用于证明验签实现正确）。
    const TEST_PUBKEY: &str = "RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3";
    /// 现代 minisign 默认（prehashed，"ED"）签名，对象为 b"test"
    const TEST_SIG_PREHASHED: &str = "untrusted comment: signature from minisign secret key\nRUQf6LRCGA9i559r3g7V1qNyJDApGip8MfqcadIgT9CuhV3EMhHoN1mGTkUidF/z7SrlQgXdy8ofjb7bNJJylDOocrCo8KLzZwo=\ntrusted comment: timestamp:1556193335\tfile:test\ny/rUw2y8/hOUYjZU71eHp/Wo1KZ40fGy2VJEDl34XMJM+TX48Ss/17u3IvIfbVR1FkZZSNCisQbuQY+bHwhEBg==\n";
    /// 旧版非预哈希（"Ed"）签名，对象为 b"test"
    const TEST_SIG_LEGACY: &str = "untrusted comment: signature from minisign secret key\nRWQf6LRCGA9i59SLOFxz6NxvASXDJeRtuZykwQepbDEGt87ig1BNpWaVWuNrm73YiIiJbq71Wi+dP9eKL8OC351vwIasSSbXxwA=\ntrusted comment: timestamp:1555779966\tfile:test\nQtKMXWyYcwdpZAlPF7tE2ENJkRd1ujvKjlj1m9RtHTBnZPa5WKU5uWRs5GoP5M/VqE81QFuMKI5k/SfNQUaOAA==\n";
    const TEST_MESSAGE: &[u8] = b"test";

    #[test]
    fn valid_signature_accepted() {
        verify_manifest_signature(TEST_PUBKEY, TEST_MESSAGE, TEST_SIG_PREHASHED)
            .expect("prehashed minisign signature must verify");
        verify_manifest_signature(TEST_PUBKEY, TEST_MESSAGE, TEST_SIG_LEGACY)
            .expect("legacy minisign signature must verify via fallback");
    }

    #[test]
    fn tampered_manifest_rejected() {
        assert!(verify_manifest_signature(TEST_PUBKEY, b"Test", TEST_SIG_PREHASHED).is_err());
        assert!(verify_manifest_signature(TEST_PUBKEY, b"Test", TEST_SIG_LEGACY).is_err());
    }

    #[test]
    fn wrong_key_rejected() {
        // 同源密钥对的另一把公钥（minisign README 示例）
        let other_key = "RWTSM+4HvQhTm9D4BpOT5d6cN0zW8KsvX8lbSFSbE9WxWpS2mEXWLmuO";
        assert!(verify_manifest_signature(other_key, TEST_MESSAGE, TEST_SIG_PREHASHED).is_err());
    }

    #[test]
    fn unsigned_and_malformed_rejected() {
        // 未配置公钥 → 拒绝（不降级）
        assert!(verify_manifest_signature("", TEST_MESSAGE, TEST_SIG_PREHASHED).is_err());
        assert!(verify_manifest_signature("   ", TEST_MESSAGE, TEST_SIG_PREHASHED).is_err());
        // 空 / 单行 / 垃圾签名 → 拒绝
        assert!(verify_manifest_signature(TEST_PUBKEY, TEST_MESSAGE, "").is_err());
        assert!(
            verify_manifest_signature(TEST_PUBKEY, TEST_MESSAGE, "untrusted comment only\n")
                .is_err()
        );
        // 篡改过的签名行（base64 合法但内容不对）→ 拒绝
        let forged = "untrusted comment: forged\nRUQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=\ntrusted comment: x\ny/rUw2y8/hOUYjZU71eHp/Wo1KZ40fGy2VJEDl34XMJM+TX48Ss/17u3IvIfbVR1FkZZSNCisQbuQY+bHwhEBg==\n".to_string();
        assert!(verify_manifest_signature(TEST_PUBKEY, TEST_MESSAGE, &forged).is_err());
    }
}
