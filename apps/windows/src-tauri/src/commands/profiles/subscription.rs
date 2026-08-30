// src-tauri/src/commands/profiles/subscription.rs
//! 订阅处理：URL 提取与脱敏、流式下载、正文归一化、
//! 订阅刷新（refresh_subscription）与启动时静默刷新。

use std::io::Write;
use std::path::Path;

use tauri::{AppHandle, Manager};
use tracing::{info, warn};

use super::files::{profile_path, temp_path_for};
use super::validate::validate_subscription_content;
use crate::util::error::{Error, Result};
use crate::util::paths::get_profiles_dir;

/// 最大下载大小（10 MB）
const MAX_DOWNLOAD_BYTES: u64 = 10 * 1024 * 1024;

/// 订阅 URL 脱敏：统一委托 `util::fetch::redact_url_for_log`（唯一实现）。
/// 仅保留 `scheme://host[:port]`，删除 userinfo/query/fragment 与完整 path
/// （path 可能携带 token，如 `/api/v1/client/<token>`）。解析失败返回 `"***"`。
pub(super) fn redact_url(url: &str) -> String {
    crate::util::fetch::redact_url_for_log(url)
}

/// 从 profile 文件内容提取订阅 URL（`# subscribe-url: <url>` 注释头）。
/// 无订阅地址的本地配置返回 None，前端据此不显示「更新」按钮。
pub(super) fn extract_subscribe_url(content: &str) -> Option<String> {
    content.lines().find_map(|line| {
        let line = line.trim();
        let rest = line
            .strip_prefix("# subscribe-url:")
            .or_else(|| line.strip_prefix("#subscribe-url:"))?;
        let url = rest.trim();
        if url.is_empty() {
            None
        } else {
            Some(url.to_string())
        }
    })
}

/// 去掉 `# subscribe-url:` 注释头，返回订阅正文（供归一化解析）。
pub(super) fn strip_subscribe_header(content: &str) -> String {
    content
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            !(t.starts_with("# subscribe-url:") || t.starts_with("#subscribe-url:"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 归一化订阅正文为 proxies-only 的 YAML 文档（Subscription Normalizer）。
/// 兼容仅含 `proxy-providers` 的现代订阅；返回 (归一化文档, 过程提示)。
pub(super) async fn normalize_subscription_body(
    app: &AppHandle,
    body: &str,
) -> Result<(String, Vec<String>)> {
    let norm = crate::util::normalizer::normalize_subscription(app, body).await?;
    for w in &norm.warnings {
        warn!("Subscription normalization: {}", w);
    }
    let mut m = serde_yaml::Mapping::new();
    m.insert(
        serde_yaml::Value::String("proxies".into()),
        serde_yaml::Value::Sequence(norm.proxies),
    );
    let doc = serde_yaml::to_string(&serde_yaml::Value::Mapping(m))?;
    Ok((doc, norm.warnings))
}

/// 脱敏订阅 URL，返回 host（含可选端口），不泄露 token/key。
///
/// - `https://user:pass@host:port/path?token=secret#frag` → `https://host:port/…`
/// - `https://host/path?token=secret` → `https://host/…`
/// - `https://host:port` → `https://host:port`
/// - 解析失败 → 返回 None（不泄露原始字符串）
pub(super) fn redact_subscribe_url(url: &str) -> Option<String> {
    reqwest::Url::parse(url).ok()?;
    Some(redact_url(url))
}

// P1 审计修复：流式大小限制 + 事务式替换
//
/// 流式下载订阅内容到临时文件（带 10MB 上限）。
/// - 响应头 Content-Length 超限：立即拒绝（不读 body）；
/// - 否则 `chunk()` 循环累计写入临时文件，累计超限即中止、删除临时文件并报错，
///   避免 `resp.text().await` 把恶意超大响应先整个读进内存。
/// - `header` 先行写入临时文件（订阅地址注释头，供「更新」命令读回 URL）；
///   其字节数计入总量上限。
pub(super) async fn download_subscription_streaming(
    app: &AppHandle,
    url: &str,
    header: &str,
    temp_path: &Path,
) -> Result<()> {
    // 拉取动作直连优先：直连不通自动切应用自身代理兜底（软件代理模式不变）
    let mut resp = crate::util::fetch::get_direct_first(app, url).await?;

    if !resp.status().is_success() {
        return Err(Error::Other(format!(
            "Failed to fetch subscription: HTTP {}",
            resp.status()
        )));
    }

    if let Some(len) = resp.content_length() {
        if len > MAX_DOWNLOAD_BYTES {
            return Err(Error::Subscription(format!(
                "Download exceeds {} bytes limit",
                MAX_DOWNLOAD_BYTES
            )));
        }
    }

    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .create_new(true)
        .open(temp_path)?;
    let mut total: u64 = 0;
    file.write_all(header.as_bytes())?;
    total += header.len() as u64;
    while let Some(chunk) = resp.chunk().await? {
        total += chunk.len() as u64;
        if total > MAX_DOWNLOAD_BYTES {
            drop(file);
            let _ = std::fs::remove_file(temp_path);
            return Err(Error::Subscription(format!(
                "Download exceeds {} bytes limit",
                MAX_DOWNLOAD_BYTES
            )));
        }
        file.write_all(&chunk)?;
    }
    file.flush()?;
    Ok(())
}

/// 订阅刷新核心逻辑（供命令与启动时静默刷新共用）：重新拉取订阅内容
/// 并事务式覆盖 profile 文件；激活中的 Profile 随后热重载生效。
pub async fn refresh_subscription(app: &AppHandle, name: &str) -> Result<()> {
    let profiles_dir = get_profiles_dir(app)?;
    let file_path = profile_path(&profiles_dir, name)?;

    if !file_path.exists() {
        return Err(Error::NotFound("Profile not found".to_string()));
    }

    let content = std::fs::read_to_string(&file_path)?;
    let url = extract_subscribe_url(&content)
        .ok_or_else(|| Error::NotFound("Profile has no subscription URL".to_string()))?;

    // 校验 scheme（与导入一致）
    let parsed = reqwest::Url::parse(&url)
        .map_err(|e| Error::InvalidArgument(format!("Invalid URL: {}", e)))?;
    match parsed.scheme() {
        "http" | "https" => {}
        _ => {
            return Err(Error::InvalidArgument(
                "URL scheme must be http or https".to_string(),
            ))
        }
    }

    // C2 SSRF 防护：parse+scheme 校验后再做禁段校验
    crate::util::fetch::validate_url(&url).await?;

    info!("Updating subscription from {}", redact_url(&url));

    // 事务第一步：下载新内容到临时文件（流式 + 大小上限）。
    // 注释头先行写入，保留订阅地址供下次更新；
    // URL 用规范化后的 parsed.as_str()，防止原始字符串反射注入。
    // 此阶段任何失败都不触碰现有文件，原订阅保持可用。
    let temp_path = temp_path_for(&file_path);
    let header = format!("# subscribe-url: {}\n", parsed.as_str());
    download_subscription_streaming(app, &url, &header, &temp_path).await?;

    // 内容校验在替换前完成；失败则清理临时文件并返回 Err
    let text = match std::fs::read_to_string(&temp_path) {
        Ok(t) => t,
        Err(e) => {
            let _ = std::fs::remove_file(&temp_path);
            return Err(e.into());
        }
    };
    if let Err(e) = validate_subscription_content(&text) {
        let _ = std::fs::remove_file(&temp_path);
        warn!(
            "Subscription update rejected for {}: {}",
            redact_url(&url),
            e
        );
        return Err(e);
    }

    // 归一化为 proxies-only 节点集（与导入一致，兼容 proxy-providers 型订阅）
    let body = strip_subscribe_header(&text);
    let (normalized, warnings) = normalize_subscription_body(app, &body).await?;
    if !warnings.is_empty() {
        warn!("Update '{}': {}", redact_url(&url), warnings.join("；"));
    }
    let final_text = format!("# subscribe-url: {}\n{}", parsed.as_str(), normalized);
    if let Err(e) = validate_subscription_content(&final_text) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(e);
    }

    // 到这里（下载/校验/归一化已完成）才提交「写临时文件 + 替换正式文件 +
    // 激活生效」复合事务（事务锁在 AppController 内部获取）。网络下载/解析
    // 耗时，不应占住全局事务锁。
    let state = app.state::<crate::AppState>();
    state
        .controller
        .commit_subscription_update(app, name, &file_path, &temp_path, &final_text)
        .await?;

    info!("Subscription updated: {}", redact_url(&url));
    Ok(())
}

// 启动时一次性订阅静默刷新
//
/// 订阅静默刷新的过期阈值：mtime 距今超过 24h 才刷新
const SUBSCRIPTION_REFRESH_AGE: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// 纯函数：从候选列表筛选需要静默刷新的 profile 名单（可测的阈值逻辑）。
///
/// - 不含 `# subscribe-url:` 头（非订阅）→ 跳过；
/// - mtime 未知 → 跳过（调用方负责对读取失败的场景 warn，避免误刷）；
/// - mtime 距 `now` 超过 `SUBSCRIPTION_REFRESH_AGE` → 刷新。
fn select_stale_subscriptions(
    candidates: Vec<(String, bool, Option<std::time::SystemTime>)>,
    now: std::time::SystemTime,
) -> Vec<String> {
    candidates
        .into_iter()
        .filter(|(_, has_url, mtime)| {
            *has_url
                && mtime
                    .map(|t| now.duration_since(t).unwrap_or_default() > SUBSCRIPTION_REFRESH_AGE)
                    .unwrap_or(false)
        })
        .map(|(name, _, _)| name)
        .collect()
}

/// 启动时一次性静默刷新过期订阅：遍历 profiles 目录，mtime 距今超过 24h
/// 且含订阅头的 .yaml 逐个串行刷新；单个失败仅 warn 不中断其他。
/// 无常驻定时器/循环——本函数执行完即返回，由调用方在启动流程末尾
/// 延迟触发一次。
pub async fn auto_refresh_stale_subscriptions(app: &AppHandle) {
    let profiles_dir = match get_profiles_dir(app) {
        Ok(dir) => dir,
        Err(e) => {
            warn!(
                "Auto subscription refresh skipped: cannot resolve profiles dir: {}",
                e
            );
            return;
        }
    };
    if !profiles_dir.exists() {
        return;
    }

    let mut candidates: Vec<(String, bool, Option<std::time::SystemTime>)> = Vec::new();
    let entries = match std::fs::read_dir(&profiles_dir) {
        Ok(entries) => entries,
        Err(e) => {
            warn!(
                "Auto subscription refresh skipped: cannot read {:?}: {}",
                profiles_dir, e
            );
            return;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("yaml") {
            continue;
        }
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        if name.is_empty() {
            continue;
        }
        // mtime 读取失败：跳过并 warn（保守处理，避免误刷）
        let mtime = match entry.metadata().and_then(|m| m.modified()) {
            Ok(t) => Some(t),
            Err(e) => {
                warn!(
                    "Auto subscription refresh: skip '{}' (mtime unavailable: {})",
                    name, e
                );
                None
            }
        };
        let has_url = std::fs::read_to_string(&path)
            .map(|c| extract_subscribe_url(&c).is_some())
            .unwrap_or(false);
        candidates.push((name, has_url, mtime));
    }

    let stale = select_stale_subscriptions(candidates, std::time::SystemTime::now());
    for name in stale {
        info!("Auto refreshing stale subscription: {}", name);
        if let Err(e) = refresh_subscription(app, &name).await {
            warn!("Auto subscription refresh failed for '{}': {}", name, e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_url_drops_query_and_userinfo_and_path() {
        // path 也可能携带 token（如 /api/v1/client/<token>），必须一并丢弃。
        assert_eq!(
            redact_url("https://example.com/path?token=secret&id=1"),
            "https://example.com/…"
        );
        assert_eq!(
            redact_url("https://user:pass@sub.example.com:8443/api/sub?token=abc#frag"),
            "https://sub.example.com:8443/…"
        );
        assert_eq!(redact_url("http://host/"), "http://host");
        assert_eq!(redact_url("https://host/path"), "https://host/…");
        // 解析失败不泄露原始串
        assert_eq!(redact_url("not a url \n bad"), "***");
    }

    // ---- 启动时一次性订阅静默刷新 ----

    /// 阈值逻辑：仅「含订阅头 + mtime 超过 24h」入选；本地配置与 mtime 未知跳过
    #[test]
    fn select_stale_filters_by_url_and_age() {
        let now = std::time::SystemTime::now();
        let fresh = now - std::time::Duration::from_secs(3600); // 1h 前：新鲜
        let stale = now - std::time::Duration::from_secs(25 * 3600); // 25h 前：过期
        let candidates = vec![
            ("fresh-sub".to_string(), true, Some(fresh)),
            ("stale-sub".to_string(), true, Some(stale)),
            ("stale-local".to_string(), false, Some(stale)), // 无订阅头
            ("no-mtime".to_string(), true, None),            // mtime 读取失败
        ];
        let out = select_stale_subscriptions(candidates, now);
        assert_eq!(out, vec!["stale-sub".to_string()]);
    }

    /// 边界：恰好 24h（等于阈值）不刷新，超过 1 秒才刷新；未来 mtime 不刷新
    #[test]
    fn select_stale_boundary_at_24h() {
        let now = std::time::SystemTime::now();
        let exact = now - SUBSCRIPTION_REFRESH_AGE;
        let over = now - SUBSCRIPTION_REFRESH_AGE - std::time::Duration::from_secs(1);
        let future = now + std::time::Duration::from_secs(600);
        let candidates = vec![
            ("exact".to_string(), true, Some(exact)),
            ("over".to_string(), true, Some(over)),
            ("future".to_string(), true, Some(future)),
        ];
        let out = select_stale_subscriptions(candidates, now);
        assert_eq!(out, vec!["over".to_string()]);
    }
}
