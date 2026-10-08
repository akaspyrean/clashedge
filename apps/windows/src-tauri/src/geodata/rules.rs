// src-tauri/src/geodata/rules.rs
//! 签名规则集更新器（审计 A4）
//!
//! 以前内置 rule-provider 让 mihomo 自行从 `…/external/main/rules/*.yaml` 浮动拉取：
//! 构建期有 commit + sha256 锁定，运行期却没有任何完整性校验，规则仓库一旦被入侵
//! 或误提交即可改写分流（例如把任意流量改成 DIRECT / REJECT）。
//!
//! 现在 rule-provider 一律是本地 `type: file`（`./rules/*.yaml`），刷新由本模块完成：
//!
//! ```text
//! rules-manifest.json + rules-manifest.json.minisig（与客户端更新共用同一把 minisign 密钥）
//!   ↓ 验签（公钥编译期注入，未配置则整体不可用）
//! 单调递增的 manifest.version（拒绝回滚 / 重放旧清单）
//!   ↓ 逐文件：路径白名单 + URL 前缀白名单 + 大小上限
//! 流式下载 → SHA256 与清单比对 → 内容结构校验
//!   ↓ 全部通过后才事务式替换（失败整体回滚）
//! 强制重载 mihomo 使 provider 重新读盘
//! ```
//!
//! 发布侧见 `scripts/release/make-rules-manifest.py`。

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use tracing::{info, warn};

use crate::geodata::download::{download_to_file, fetch_small, replace_all, sha256_of_file};
use crate::util::error::{Error, Result};

/// 规则清单地址（外部规则仓库根目录）
pub const RULES_MANIFEST_URL: &str =
    "https://raw.githubusercontent.com/akaspyrean/external/main/rules-manifest.json";

/// 清单中文件 URL 允许的前缀（commit 固定的 raw 地址或 jsDelivr 镜像）
const ALLOWED_URL_PREFIXES: &[&str] = &[
    "https://raw.githubusercontent.com/akaspyrean/external/",
    "https://cdn.jsdelivr.net/gh/akaspyrean/external@",
];

/// 允许由清单更新的、位于 Data 根目录的 geodata 文件名
const ROOT_FILES: &[&str] = &["GeoIP.dat", "GeoSite.dat", "Country.mmdb"];

const MAX_MANIFEST_BYTES: u64 = 256 * 1024;
const MAX_RULE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_GEO_BYTES: u64 = 200 * 1024 * 1024;
const FILE_DEADLINE: Duration = Duration::from_secs(600);

#[derive(Debug, Clone, Deserialize)]
pub struct RuleFile {
    /// 相对 Data 的目标路径：`rules/<name>.yaml` 或 `GeoIP.dat` / `GeoSite.dat` / `Country.mmdb`
    pub path: String,
    pub url: String,
    pub sha256: String,
    #[serde(default)]
    pub size: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RulesManifest {
    /// 单调递增版本（发布脚本使用 `YYYYMMDDHHMM`）
    pub version: u64,
    #[serde(default)]
    pub released_at: u64,
    pub files: Vec<RuleFile>,
}

/// 更新结果
#[derive(Debug, Default)]
pub struct RulesOutcome {
    pub version: u64,
    pub updated: Vec<String>,
    pub up_to_date: bool,
}

/// 目标路径白名单：`rules/<[a-z0-9_-]+>.yaml` 或固定的 geodata 文件名。
pub fn validate_target_path(path: &str) -> Result<PathBuf> {
    if ROOT_FILES.contains(&path) {
        return Ok(PathBuf::from(path));
    }
    if let Some(name) = path.strip_prefix("rules/") {
        if let Some(stem) = name.strip_suffix(".yaml") {
            if !stem.is_empty()
                && stem.len() <= 64
                && stem
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
            {
                return Ok(PathBuf::from("rules").join(name));
            }
        }
    }
    Err(Error::Other(format!(
        "rules manifest: path '{}' is not allowed",
        path
    )))
}

/// 文件 URL 必须落在白名单前缀下。
pub fn validate_file_url(url: &str) -> Result<()> {
    if ALLOWED_URL_PREFIXES.iter().any(|p| url.starts_with(p)) && !url.contains("..") {
        Ok(())
    } else {
        Err(Error::Other(format!(
            "rules manifest: url '{}' is outside the allowed hosts",
            crate::util::fetch::redact_url_for_log(url)
        )))
    }
}

fn validate_sha(sha: &str) -> Result<()> {
    if sha.len() == 64 && sha.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(Error::Other("rules manifest: invalid sha256".to_string()))
    }
}

/// 内容结构校验：规则集必须是带非空 `payload` 序列的 YAML；geodata 做最小结构检查。
pub fn validate_content(rel: &str, path: &Path) -> Result<()> {
    if rel.ends_with(".yaml") {
        let text = std::fs::read_to_string(path)
            .map_err(|e| Error::Other(format!("{}: not valid UTF-8 text: {}", rel, e)))?;
        let doc: serde_yaml::Value = serde_yaml::from_str(&text)
            .map_err(|e| Error::Other(format!("{}: invalid YAML: {}", rel, e)))?;
        let ok = doc
            .get("payload")
            .and_then(|p| p.as_sequence())
            .map(|s| !s.is_empty())
            .unwrap_or(false);
        if !ok {
            return Err(Error::Other(format!(
                "{}: missing non-empty `payload`",
                rel
            )));
        }
        Ok(())
    } else {
        crate::geodata::updater::validate_geodata_file(path, rel)
    }
}

fn state_path(data_dir: &Path) -> PathBuf {
    data_dir.join("rules-state.json")
}

fn read_local_version(data_dir: &Path) -> u64 {
    std::fs::read_to_string(state_path(data_dir))
        .ok()
        .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
        .and_then(|v| v.get("version").and_then(|x| x.as_u64()))
        .unwrap_or(0)
}

fn write_local_version(data_dir: &Path, version: u64) {
    let body = serde_json::json!({ "version": version });
    let _ = crate::util::atomic::atomic_write(&state_path(data_dir), body.to_string().as_bytes());
}

/// 更新规则集 / geodata（签名清单驱动）。
pub async fn update_rules(app: &tauri::AppHandle) -> Result<RulesOutcome> {
    if crate::update::UPDATE_PUBLIC_KEY.trim().is_empty() {
        return Err(Error::Other(
            "签名规则更新不可用：客户端未内置验签公钥（构建配置缺失）".to_string(),
        ));
    }
    let data_dir = crate::util::paths::get_app_data_dir(app)?;

    let manifest_bytes = fetch_small(app, RULES_MANIFEST_URL, MAX_MANIFEST_BYTES).await?;
    let sig = fetch_small(app, &format!("{}.minisig", RULES_MANIFEST_URL), 16 * 1024).await?;
    let sig_text =
        String::from_utf8(sig).map_err(|_| Error::Other("signature is not UTF-8".into()))?;
    crate::update::verify_manifest_signature(
        crate::update::UPDATE_PUBLIC_KEY,
        &manifest_bytes,
        &sig_text,
    )?;
    let manifest: RulesManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|e| Error::Other(format!("rules manifest invalid: {}", e)))?;

    let local = read_local_version(&data_dir);
    if manifest.version < local {
        return Err(Error::Other(format!(
            "rules manifest v{} is older than the installed v{}; rejected (rollback protection)",
            manifest.version, local
        )));
    }

    // 逐文件预检（全部合法才开始下载）
    let mut plan: Vec<(RuleFile, PathBuf)> = Vec::new();
    for f in &manifest.files {
        let rel = validate_target_path(&f.path)?;
        validate_file_url(&f.url)?;
        validate_sha(&f.sha256)?;
        plan.push((f.clone(), data_dir.join(rel)));
    }

    // 下载只针对"本地哈希不同"的文件
    let mut downloaded: Vec<(PathBuf, PathBuf, String)> = Vec::new(); // (新文件, 目标, 相对路径)
    let result: Result<()> = async {
        for (f, target) in &plan {
            let want = f.sha256.to_ascii_lowercase();
            if sha256_of_file(target).as_deref() == Some(want.as_str()) {
                continue;
            }
            if let Some(dir) = target.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let tmp = {
                let mut n = target.file_name().unwrap_or_default().to_os_string();
                n.push(".download");
                target.with_file_name(n)
            };
            let max = if f.path.ends_with(".yaml") {
                MAX_RULE_BYTES
            } else {
                MAX_GEO_BYTES
            };
            let (size, got) = download_to_file(app, &f.url, &tmp, max, FILE_DEADLINE).await?;
            if got != want {
                let _ = std::fs::remove_file(&tmp);
                return Err(Error::Other(format!(
                    "{}: SHA256 mismatch (expected {}, got {})",
                    f.path, want, got
                )));
            }
            if f.size != 0 && f.size != size {
                let _ = std::fs::remove_file(&tmp);
                return Err(Error::Other(format!(
                    "{}: size differs from manifest",
                    f.path
                )));
            }
            if let Err(e) = validate_content(&f.path, &tmp) {
                let _ = std::fs::remove_file(&tmp);
                return Err(e);
            }
            downloaded.push((tmp, target.clone(), f.path.clone()));
        }
        Ok(())
    }
    .await;
    if let Err(e) = result {
        for (tmp, _, _) in &downloaded {
            let _ = std::fs::remove_file(tmp);
        }
        return Err(e);
    }

    let mut outcome = RulesOutcome {
        version: manifest.version,
        ..Default::default()
    };
    if downloaded.is_empty() {
        outcome.up_to_date = true;
        write_local_version(&data_dir, manifest.version);
        info!("Rules already up to date (v{})", manifest.version);
        return Ok(outcome);
    }

    let pairs: Vec<(PathBuf, PathBuf)> = downloaded
        .iter()
        .map(|(n, t, _)| (n.clone(), t.clone()))
        .collect();
    replace_all(&pairs)?;
    outcome.updated = downloaded.into_iter().map(|(_, _, rel)| rel).collect();
    write_local_version(&data_dir, manifest.version);
    info!(
        "Rules updated to v{} (released_at {}, {} files): {:?}",
        manifest.version,
        manifest.released_at,
        outcome.updated.len(),
        outcome.updated
    );
    if let Err(e) = reload_core(app).await {
        warn!("Rules updated but core reload failed: {}", e);
    }
    Ok(outcome)
}

/// 强制重载核心（provider / geodata 文件被替换后必须重新读盘）。
pub async fn reload_core(app: &tauri::AppHandle) -> Result<()> {
    use tauri::Manager;
    let state = app.state::<crate::AppState>();
    if let Some(core) = state.core_manager.get() {
        if core.is_running() {
            core.reload_config_force().await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_paths_are_whitelisted() {
        assert!(validate_target_path("rules/ai.yaml").is_ok());
        assert!(validate_target_path("rules/my-set_2.yaml").is_ok());
        assert!(validate_target_path("GeoIP.dat").is_ok());
        for bad in [
            "../config.yaml",
            "rules/../config.yaml",
            "rules/AI.yaml",
            "rules/a.yml",
            "rules/sub/x.yaml",
            "rules/.yaml",
            "config.yaml",
            "C:/x.dat",
            "geoip.dat ",
        ] {
            assert!(validate_target_path(bad).is_err(), "{}", bad);
        }
    }

    #[test]
    fn urls_must_be_on_allowed_hosts() {
        assert!(validate_file_url(
            "https://raw.githubusercontent.com/akaspyrean/external/cbee39d359c3e466f03cf485ecd112fd63a5872c/rules/ai.yaml"
        )
        .is_ok());
        assert!(validate_file_url(
            "https://cdn.jsdelivr.net/gh/akaspyrean/external@abc/rules/ai.yaml"
        )
        .is_ok());
        assert!(
            validate_file_url("https://raw.githubusercontent.com/evil/external/main/x.yaml")
                .is_err()
        );
        assert!(validate_file_url("https://example.com/rules/ai.yaml").is_err());
        assert!(validate_file_url(
            "https://raw.githubusercontent.com/akaspyrean/external/../../evil/x.yaml"
        )
        .is_err());
    }

    #[test]
    fn rule_yaml_content_requires_payload() {
        let d = std::env::temp_dir().join(format!("clashedge-rules-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let ok = d.join("ok.yaml");
        std::fs::write(&ok, "payload:\n  - DOMAIN,example.com\n").unwrap();
        assert!(validate_content("rules/ok.yaml", &ok).is_ok());
        let empty = d.join("empty.yaml");
        std::fs::write(&empty, "payload: []\n").unwrap();
        assert!(validate_content("rules/empty.yaml", &empty).is_err());
        let html = d.join("html.yaml");
        std::fs::write(&html, "<html><body>404</body></html>").unwrap();
        assert!(validate_content("rules/html.yaml", &html).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn manifest_parses_and_version_state_roundtrips() {
        let m: RulesManifest = serde_json::from_str(
            r#"{"version":202610080000,"files":[{"path":"rules/ai.yaml","url":"https://raw.githubusercontent.com/akaspyrean/external/abc/rules/ai.yaml","sha256":"00","size":1}]}"#,
        )
        .unwrap();
        assert_eq!(m.version, 202610080000);
        assert_eq!(m.files.len(), 1);
        let d = std::env::temp_dir().join(format!("clashedge-rules-state-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        assert_eq!(read_local_version(&d), 0);
        write_local_version(&d, 42);
        assert_eq!(read_local_version(&d), 42);
        let _ = std::fs::remove_dir_all(&d);
    }
}
