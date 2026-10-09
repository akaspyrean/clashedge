// src-tauri/src/commands/profiles/subscription.rs
//! 订阅处理：URL 提取与脱敏、流式下载、正文归一化、
//! 订阅刷新（refresh_subscription）与启动时静默刷新。

use std::io::Write;
use std::path::Path;

use tauri::{AppHandle, Manager};
use tracing::{info, warn};

use super::files::{profile_path, temp_path_for};
use super::validate::{validate_profile_strict, validate_subscription_content};
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

/// 去掉 `# subscribe-url:` / `# subscription-userinfo:` / `# subscribe-user-agent:`
/// 注释头，返回订阅正文（供归一化解析；Base64 订阅不能夹杂注释行）。
pub(super) fn strip_subscribe_header(content: &str) -> String {
    content
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            !(t.starts_with("# subscribe-url:")
                || t.starts_with("#subscribe-url:")
                || t.starts_with("# subscription-userinfo:")
                || t.starts_with("# subscribe-user-agent:")
                || t.starts_with("#subscribe-user-agent:"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 只读取 profile 文件开头的一小段（头注释都在文件最前面）。
/// `list_profiles` / 静默刷新不再把最大 10 MB 的订阅整个读进内存只为取一行 URL。
pub(super) fn read_profile_head(path: &Path) -> String {
    use std::io::Read;
    let Ok(mut f) = std::fs::File::open(path) else {
        return String::new();
    };
    let mut buf = vec![0u8; 4096];
    let n = f.read(&mut buf).unwrap_or(0);
    buf.truncate(n);
    String::from_utf8_lossy(&buf).to_string()
}

/// 从 profile 头注释提取 `Subscription-Userinfo`（`upload=..; download=..; total=..; expire=..`）。
pub(super) fn extract_subscription_userinfo(content: &str) -> Option<String> {
    content.lines().find_map(|line| {
        let rest = line.trim().strip_prefix("# subscription-userinfo:")?;
        let v = rest.trim();
        if v.is_empty() {
            None
        } else {
            Some(v.to_string())
        }
    })
}

/// 从 profile 头注释提取自定义 User-Agent（`# subscribe-user-agent: <ua>`，
/// 审计 B7：部分服务端按 UA 决定返回格式，允许为单个订阅覆盖）。
pub(super) fn extract_subscribe_user_agent(content: &str) -> Option<String> {
    content.lines().find_map(|line| {
        let line = line.trim();
        let rest = line
            .strip_prefix("# subscribe-user-agent:")
            .or_else(|| line.strip_prefix("#subscribe-user-agent:"))?;
        let v = rest.trim();
        if v.is_empty() {
            None
        } else {
            Some(v.to_string())
        }
    })
}

/// 自定义 User-Agent 白名单净化：仅保留可见 ASCII（0x20–0x7E），删除控制字符
/// 与 DEL（防止头注释注入换行、HTTP 头注入与 `HeaderValue` 构造失败），
/// 限长 200；trim 后非空才有效，空/全不可用返回 None（回到内置默认 UA）。
pub(super) fn sanitize_user_agent(raw: &str) -> Option<String> {
    let cleaned: String = raw
        .chars()
        .filter(|c| ('\x20'..='\x7e').contains(c))
        .take(200)
        .collect();
    let cleaned = cleaned.trim().to_string();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

/// 响应头 `Subscription-Userinfo` 白名单净化：仅保留数字 / 字母 / `=;. _-`，
/// 限长，防止头注释注入换行或超长内容。
pub(super) fn sanitize_userinfo(raw: &str) -> Option<String> {
    let cleaned: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '=' | ';' | '.' | ' ' | '_' | '-'))
        .take(200)
        .collect();
    let cleaned = cleaned.trim().to_string();
    if cleaned.contains('=') {
        Some(cleaned)
    } else {
        None
    }
}

/// 组装 profile 头注释（订阅地址 + 可选流量信息 + 可选自定义 UA）。
pub(super) fn build_header(url: &str, userinfo: Option<&str>, user_agent: Option<&str>) -> String {
    let mut h = format!("# subscribe-url: {}\n", url);
    if let Some(u) = userinfo {
        h.push_str(&format!("# subscription-userinfo: {}\n", u));
    }
    if let Some(ua) = user_agent {
        h.push_str(&format!("# subscribe-user-agent: {}\n", ua));
    }
    h
}

/// 更新 profile 头注释中的自定义 User-Agent 行（纯函数，可测）：
/// - `Some(ua)`：替换既有行，或插入到 `# subscribe-url:` 行之后（无则插到最前）；
/// - `None`：移除该行（回到内置默认 UA）。
///
/// 保留原文的结尾换行风格。
pub(super) fn apply_header_user_agent(content: &str, user_agent: Option<&str>) -> String {
    let ends_with_newline = content.ends_with('\n');
    let mut lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
    let ua_idx = lines.iter().position(|l| {
        let t = l.trim_start();
        t.starts_with("# subscribe-user-agent:") || t.starts_with("#subscribe-user-agent:")
    });
    match (user_agent, ua_idx) {
        (Some(ua), Some(i)) => lines[i] = format!("# subscribe-user-agent: {}", ua),
        (Some(ua), None) => {
            let insert_at = lines
                .iter()
                .position(|l| {
                    let t = l.trim_start();
                    t.starts_with("# subscribe-url:") || t.starts_with("#subscribe-url:")
                })
                .map(|i| i + 1)
                .unwrap_or(0);
            lines.insert(insert_at, format!("# subscribe-user-agent: {}", ua));
        }
        (None, Some(i)) => {
            lines.remove(i);
        }
        (None, None) => {}
    }
    let mut out = lines.join("\n");
    if ends_with_newline {
        out.push('\n');
    }
    out
}

/// 归一化订阅正文为 proxies-only 的 YAML 文档（Subscription Normalizer）。
/// 兼容仅含 `proxy-providers` 的现代订阅；返回 (归一化文档, 过程提示)。
/// 归一化后同步检测节点区域（GeoIP + DoH），写入 profiles/<name>.region.json。
pub(super) async fn normalize_subscription_body(
    app: &AppHandle,
    profile_name: &str,
    body: &str,
) -> Result<(String, Vec<String>)> {
    let norm = crate::util::normalizer::normalize_subscription(app, body).await?;
    for w in &norm.warnings {
        warn!("Subscription normalization: {}", w);
    }
    // 区域检测：失败只降级为空 map，不阻断导入/刷新。
    // 在序列化前用 norm.proxies 检测，避免重复解析。
    let _regions = crate::util::region::detect_and_persist(app, profile_name, &norm.proxies).await;
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
///
/// 失败（含下载中途的读写错误）时一律删除临时文件，不留 `.tmp.*` 残留。
/// 成功返回响应头里的（已净化）`Subscription-Userinfo`。
pub(super) async fn download_subscription_streaming(
    app: &AppHandle,
    url: &str,
    header: &str,
    temp_path: &Path,
    user_agent: Option<&str>,
) -> Result<Option<String>> {
    let result = download_subscription_inner(app, url, header, temp_path, user_agent).await;
    if result.is_err() {
        let _ = std::fs::remove_file(temp_path);
    }
    result
}

async fn download_subscription_inner(
    app: &AppHandle,
    url: &str,
    header: &str,
    temp_path: &Path,
    user_agent: Option<&str>,
) -> Result<Option<String>> {
    // 拉取动作直连优先：直连不通自动切应用自身代理兜底（软件代理模式不变）。
    // 单个订阅可覆盖 User-Agent（审计 B7）；None 用内置 mihomo 兼容 UA。
    let mut resp = crate::util::fetch::get_direct_first_with_ua(app, url, user_agent).await?;

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

    let userinfo = resp
        .headers()
        .get("subscription-userinfo")
        .and_then(|v| v.to_str().ok())
        .and_then(sanitize_userinfo);

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
    Ok(userinfo)
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
    // 单个订阅的自定义 UA（审计 B7）：头注释携带，随刷新持久保留
    let custom_ua = extract_subscribe_user_agent(&content);

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

    // C2 SSRF 防护：parse+scheme 校验后再做禁段校验（感知 TUN + fake-ip，见 A3）
    crate::util::fetch::validate_url_app(app, &url).await?;

    info!("Updating subscription from {}", redact_url(&url));

    // 事务第一步：下载新内容到临时文件（流式 + 大小上限）。
    // 注释头先行写入，保留订阅地址供下次更新；
    // URL 用规范化后的 parsed.as_str()，防止原始字符串反射注入。
    // 此阶段任何失败都不触碰现有文件，原订阅保持可用。
    let temp_path = temp_path_for(&file_path);
    // RAII：任何提前返回（含 `?`）都会清理临时文件（成功提交后文件已被 rename 走，删除为空操作）
    let _temp_guard = crate::util::temp::TempFile::new(temp_path.clone());
    let header = build_header(parsed.as_str(), None, custom_ua.as_deref());
    let userinfo =
        download_subscription_streaming(app, &url, &header, &temp_path, custom_ua.as_deref())
            .await?;

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
    let (normalized, warnings) = normalize_subscription_body(app, name, &body).await?;
    if !warnings.is_empty() {
        warn!("Update '{}': {}", redact_url(&url), warnings.join("；"));
    }
    let final_text = format!(
        "{}{}",
        build_header(parsed.as_str(), userinfo.as_deref(), custom_ua.as_deref()),
        normalized
    );
    if let Err(e) = validate_profile_strict(&final_text) {
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
        let has_url = extract_subscribe_url(&read_profile_head(&path)).is_some();
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

    #[test]
    fn userinfo_is_sanitized_and_roundtrips_through_header() {
        let raw = "upload=1; download=2; total=3; expire=4\r\n# evil: x";
        let clean = sanitize_userinfo(raw).unwrap();
        assert!(!clean.contains('\n') && !clean.contains('#') && !clean.contains(':'));
        let header = build_header("https://h.example/sub", Some(&clean), None);
        assert_eq!(
            extract_subscribe_url(&header).as_deref(),
            Some("https://h.example/sub")
        );
        assert_eq!(
            extract_subscription_userinfo(&header).as_deref(),
            Some(clean.as_str())
        );
        // 头注释不进入正文
        let body = strip_subscribe_header(&format!("{}proxies: []\n", header));
        assert!(!body.contains("subscription-userinfo") && body.contains("proxies"));
        assert!(sanitize_userinfo("no equals sign").is_none());
    }

    // ---- 单个订阅自定义 User-Agent（审计 B7）----

    /// 净化：删除控制字符/换行（防头注释与 HTTP 头注入）、限长、trim；
    /// 空 / 全不可用返回 None。
    #[test]
    fn user_agent_is_sanitized_and_rejects_unusable() {
        let raw = "ClashEdge/1.0\r\nX-Evil: injected";
        let clean = sanitize_user_agent(raw).unwrap();
        assert!(!clean.contains('\n') && !clean.contains('\r'));
        assert_eq!(clean, "ClashEdge/1.0X-Evil: injected");
        // 非可见 ASCII（DEL / 控制字符）被删除
        assert_eq!(
            sanitize_user_agent("a\u{7f}b\u{1}c").as_deref(),
            Some("abc")
        );
        // 限长 200
        let long = "x".repeat(300);
        assert_eq!(sanitize_user_agent(&long).map(|s| s.len()), Some(200));
        // 空与全不可用
        assert!(sanitize_user_agent("").is_none());
        assert!(sanitize_user_agent("   ").is_none());
        assert!(sanitize_user_agent("\r\n\u{1}").is_none());
    }

    /// 头注释 roundtrip：build → extract → strip（UA 行不进入正文）
    #[test]
    fn user_agent_roundtrips_through_header() {
        let header = build_header(
            "https://h.example/sub",
            Some("upload=1; total=2"),
            Some("ClashEdge/1.0 (custom)"),
        );
        assert_eq!(
            extract_subscribe_url(&header).as_deref(),
            Some("https://h.example/sub")
        );
        assert_eq!(
            extract_subscribe_user_agent(&header).as_deref(),
            Some("ClashEdge/1.0 (custom)")
        );
        let body = strip_subscribe_header(&format!("{}proxies: []\n", header));
        assert!(!body.contains("subscribe-user-agent"));
        assert!(!body.contains("subscription-userinfo"));
        assert!(body.contains("proxies"));
        // 无 UA 行：extract 返回 None
        assert!(extract_subscribe_user_agent("# subscribe-url: https://h/").is_none());
    }

    /// apply_header_user_agent：替换既有行 / 插入到 subscribe-url 之后 / None 移除，
    /// 且保留结尾换行风格。
    #[test]
    fn header_user_agent_replace_insert_and_remove() {
        // 替换既有行
        let doc = "# subscribe-url: https://h/\n# subscribe-user-agent: old\nproxies: []\n";
        let out = apply_header_user_agent(doc, Some("new"));
        assert_eq!(
            out,
            "# subscribe-url: https://h/\n# subscribe-user-agent: new\nproxies: []\n"
        );
        // 插入到 subscribe-url 行之后
        let doc = "# subscribe-url: https://h/\nproxies: []\n";
        let out = apply_header_user_agent(doc, Some("custom"));
        assert_eq!(
            out,
            "# subscribe-url: https://h/\n# subscribe-user-agent: custom\nproxies: []\n"
        );
        // 移除既有行
        let out = apply_header_user_agent(
            "# subscribe-url: https://h/\n# subscribe-user-agent: old\nproxies: []\n",
            None,
        );
        assert_eq!(out, "# subscribe-url: https://h/\nproxies: []\n");
        // 无行可移除：原文不变
        assert_eq!(
            apply_header_user_agent("proxies: []\n", None),
            "proxies: []\n"
        );
        // 无 subscribe-url 行时插到最前
        assert_eq!(
            apply_header_user_agent("proxies: []", Some("u")),
            "# subscribe-user-agent: u\nproxies: []"
        );
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
