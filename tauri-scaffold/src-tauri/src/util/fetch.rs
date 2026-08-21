// src-tauri/src/util/fetch.rs
//! 拉取助手：直连优先、直连不通自动切换到应用自身代理兜底
//!
//! 需求：订阅更新、配置更新（geodata）这类"拉取动作"默认走直连；
//! 直连不通时自动改走应用自身 mihomo 混合端口（`127.0.0.1:{mixed_port}`）
//! 作为 HTTP 代理重试。软件本身的代理模式（rule/global/direct）不受影响。
//!
//! 判定"直连不通"：直连请求连接失败/超时，或返回非 2xx 状态码
//! （服务器对直连 IP 限流/屏蔽时常见，走代理可绕过）。
//!
//! 安全（SSRF 防护）：所有拉取目标 URL 必须通过 `validate_url` 校验
//! （拒绝非 http/https、localhost/.local、回环/私网/链路本地等禁段），
//! 并对直连与代理兜底两个 client 统一施加手动重定向处理——跳转前逐跳
//! 做完整异步校验（含 DNS 禁段检查），命中禁段即中止，且限制最大跳数。
//! 校验的是**目标 URL**，不是代理地址。
//!
//! TOCTOU 收敛：`validate_url` 解析 DNS 后返回已校验的地址列表，
//! `get_direct_first` 用 `reqwest::Client::resolve()` 将主机名钉定到
//! 已校验的 IP，避免"校验一次 DNS、连接时重新解析"的窗口。重定向
//! 到新主机名时同样先异步校验再钉定连接。

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use reqwest::header::USER_AGENT;
use reqwest::redirect::Policy;
use reqwest::Proxy;
use reqwest::Url;
use tauri::{AppHandle, Manager};
use tracing::{info, warn};

use crate::util::error::{Error, Result};

/// 单次拉取超时（与既有订阅拉取行为一致）
const TIMEOUT: Duration = Duration::from_secs(30);

/// 最大重定向跳数（防重定向链被当作跳板无限转发/探测内网）
const MAX_REDIRECTS: usize = 3;

/// 拉取请求 User-Agent（部分订阅/下载服务器要求非空 UA）
const USER_AGENT_VALUE: &str = "ClashEdge/0.8.5";

/// SSRF 防护：校验目标 URL 是否允许被拉取（异步，含非 IP 主机名 DNS 检查）。
///
/// 拒绝：
/// - 非 http/https scheme；
/// - `localhost` / `*.localhost` / `*.local` 主机名；
/// - 字面 IP 落在回环 / 私网 / 链路本地 / 未指定等禁段；
/// - 非 IP 主机名解析出的地址中**只要存在**任一禁段地址即拒绝
///   （防止 DNS rebinding / 混合解析绕过）；
/// - DNS 解析失败即拒绝（不保守放行，防止 DNS 故障时放行内网地址）。
///
/// 返回已校验的解析地址列表（供调用方钉定连接，关闭 TOCTOU）。
pub async fn validate_url(url: &str) -> Result<Vec<SocketAddr>> {
    validate_url_sync(url)?;
    let parsed = Url::parse(url)
        .map_err(|e| Error::InvalidArgument(format!("Invalid URL: {}", e)))?;
    let Some(host) = parsed.host_str() else {
        return Err(Error::InvalidArgument("URL has no host".to_string()));
    };
    let port = parsed.port().unwrap_or(if parsed.scheme() == "https" { 443 } else { 80 });

    // 字面 IP 已在 sync 阶段校验；这里返回单元素列表供钉定
    if let Some(ip) = parse_host_ip(host) {
        return Ok(vec![SocketAddr::new(ip, port)]);
    }

    // 非 IP 主机名：DNS 解析后逐个校验
    let addrs = tokio::net::lookup_host((host, port))
        .await
        .map_err(|e| {
            // DNS 解析失败不得默认放行——防止 DNS 故障/劫持时放行内网地址
            Error::InvalidArgument(format!(
                "URL host '{}' DNS lookup failed ({}); rejected (fail-closed)",
                host, e
            ))
        })?;
    let addrs: Vec<SocketAddr> = addrs.collect();
    if addrs.is_empty() {
        return Err(Error::InvalidArgument(format!(
            "URL host '{}' resolved to no addresses; rejected",
            host
        )));
    }
    // 只要存在任一禁段地址即拒绝（DNS rebinding / 混合解析防护）
    for a in &addrs {
        if is_denied_ip(a.ip()) {
            return Err(Error::InvalidArgument(format!(
                "URL host '{}' resolves to a blocked address ({}); rejected",
                host,
                a.ip()
            )));
        }
    }
    Ok(addrs)
}

/// 同步 URL 校验（scheme / 主机名 / 字面 IP）。
/// 供重定向目标预检与 `validate_url` 的字面量阶段调用（策略回调是同步的，
/// 无法 await DNS，因此只做字面量校验）。
fn validate_url_sync(url: &str) -> Result<()> {
    let parsed = Url::parse(url)
        .map_err(|e| Error::InvalidArgument(format!("Invalid URL: {}", e)))?;
    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        return Err(Error::InvalidArgument(format!(
            "URL scheme '{}' must be http or https",
            scheme
        )));
    }
    let Some(host) = parsed.host_str() else {
        return Err(Error::InvalidArgument("URL has no host".to_string()));
    };
    let host_lower = host.to_ascii_lowercase();
    if host_lower == "localhost"
        || host_lower.ends_with(".localhost")
        || host_lower.ends_with(".local")
    {
        return Err(Error::InvalidArgument(format!(
            "URL host '{}' is not allowed",
            host
        )));
    }
    if let Some(ip) = parsed.host_str().and_then(parse_host_ip) {
        if is_denied_ip(ip) {
            return Err(Error::InvalidArgument(format!(
                "URL host '{}' is a blocked address",
                ip
            )));
        }
    }
    Ok(())
}

/// 从 URL host 字符串解析字面 IP。
/// `Url::host_str()` 对 IPv6 字面量会保留原文的方括号（如 `[::1]`），
/// 直接 `parse::<IpAddr>()` 会失败，这里先剥离方括号。
fn parse_host_ip(host: &str) -> Option<IpAddr> {
    host.strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host)
        .parse()
        .ok()
}

/// 禁段 IP 判定：回环 / 私网 / 链路本地 / 未指定。
/// 当前工具链的 `IpAddr` 上没有 `is_private`/`is_link_local`，按 V4/V6 分别判定；
/// IPv6 私网等价物为唯一本地地址（fc00::/7）、链路本地为 fe80::/10。
/// IPv4-mapped（::ffff:a.b.c.d）必须按内嵌 V4 判定，否则可经
/// `http://[::ffff:127.0.0.1]:9090/` 绕过回环/私网封锁。
fn is_denied_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_unspecified()
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return v4.is_loopback()
                    || v4.is_private()
                    || v4.is_link_local()
                    || v4.is_unspecified();
            }
            v6.is_loopback()
                || v6.is_unique_local()
                || v6.is_unicast_link_local()
                || v6.is_unspecified()
        }
    }
}

/// 手动重定向策略：**不自动跟随**，返回重定向信息让调用方做完整异步校验
/// 后再决定是否继续。自动策略的回调是同步的无法做 DNS 校验，故改用手动。
fn no_redirect_policy() -> Policy {
    Policy::none()
}

/// 直连优先、代理兜底地发起 GET 请求。
///
/// 流程：
/// 1. 先用直连 client（`no_proxy`）请求目标 URL；
/// 2. 直连失败（连接失败/超时/非 2xx）则改用应用自身 mihomo 混合端口
///    （`http://127.0.0.1:{mixed_port}`）作为 HTTP 代理重试一次；
/// 3. 返回最终 response（调用方负责消费 body / 检查状态码）。
///
/// 返回的 response 状态码不保证是 2xx——调用方需自行判定；
/// 但若直连已拿到 2xx，则不会发起代理重试。
///
/// SSRF + TOCTOU：目标 URL 必须先通过 `validate_url`（含 DNS 禁段检查，
/// 解析失败即拒绝）；校验返回的已验地址用 `resolve()` 钉定到客户端，
/// 避免连接时重新解析。重定向到新主机名时逐跳做完整异步校验再钉定。
pub async fn get_direct_first(app: &AppHandle, url: &str) -> Result<reqwest::Response> {
    // SSRF 防护：目标 URL 必须通过校验（含 DNS 禁段检查，返回已验地址）
    let resolved = validate_url(url).await?;

    // 1. 直连尝试（no_proxy：忽略系统代理/环境代理，强制直连）
    let direct = build_client_with_resolved(url, &resolved)?
        .no_proxy()
        .redirect(no_redirect_policy())
        .build()?;
    match send_and_follow(&direct, url, USER_AGENT_VALUE).await {
        Ok(resp) if resp.status().is_success() => return Ok(resp),
        Ok(resp) => {
            warn!(
                "Direct fetch got non-success status {} for {}; retrying via proxy",
                resp.status(),
                url
            );
        }
        Err(e) => {
            warn!(
                "Direct fetch failed for {}: {}; retrying via proxy",
                url, e
            );
        }
    }

    // 2. 代理兜底：应用自身 mihomo 混合端口
    let proxy_url = local_proxy_url(app);
    info!("Fetching {} via local proxy {}", url, proxy_url);
    let proxied = build_client_with_resolved(url, &resolved)?
        .proxy(Proxy::all(&proxy_url)?)
        .redirect(no_redirect_policy())
        .build()?;
    send_and_follow(&proxied, url, USER_AGENT_VALUE)
        .await
        .map_err(Error::from)
}

/// 从 URL 提取主机名与已校验地址，构建带 `resolve()` 钉定的 ClientBuilder。
/// 钉定后 reqwest 连接时直接用已校验的 IP，不再重新解析 DNS（关闭 TOCTOU）。
fn build_client_with_resolved(url: &str, resolved: &[SocketAddr]) -> Result<reqwest::ClientBuilder> {
    let parsed = Url::parse(url)
        .map_err(|e| Error::InvalidArgument(format!("Invalid URL: {}", e)))?;
    let mut builder = reqwest::Client::builder().timeout(TIMEOUT);
    if let Some(host) = parsed.host_str() {
        if let Some(first) = resolved.first() {
            let host_no_brackets = host
                .strip_prefix('[')
                .and_then(|h| h.strip_suffix(']'))
                .unwrap_or(host);
            builder = builder.resolve(host_no_brackets, *first);
        }
    }
    Ok(builder)
}

/// 发送请求并手动处理重定向：每跳做完整异步 SSRF 校验（含 DNS）后
/// 用钉定连接跟随，避免自动重定向的同步回调无法做 DNS 校验。
async fn send_and_follow(
    client: &reqwest::Client,
    start_url: &str,
    ua: &str,
) -> Result<reqwest::Response> {
    let mut current_url = start_url.to_string();
    for _hop in 0..=MAX_REDIRECTS {
        // 每跳（含首跳）都校验：首跳已由调用方校验，重定向到的新 URL 需要新校验
        let resolved = if _hop == 0 {
            // 首跳：调用方已校验过，直接复用。但 build_client_with_resolved 已钉定，
            // 这里 client 是已构建好的，直接发送即可。
            Vec::new()
        } else {
            validate_url(&current_url).await?
        };
        // 对于重定向跳，需要用新 URL 的已验地址重新构建请求；
        // 但 client 是共享的（已钉定首跳主机）。重定向到新主机名时，
        // 我们用 client.get(new_url) 发送——reqwest 会对新主机重新解析。
        // 为关闭 TOCTOU，重定向到新主机时改用独立钉定 client。
        let req_client = if _hop == 0 {
            // 首跳：client 已由调用方钉定
            client.clone()
        } else if resolved.is_empty() {
            client.clone()
        } else {
            build_client_with_resolved(&current_url, &resolved)?
                .redirect(no_redirect_policy())
                .build()?
        };

        let resp = req_client
            .get(&current_url)
            .header(USER_AGENT, ua)
            .send()
            .await?;

        if resp.status().is_success() {
            return Ok(resp);
        }
        if !resp.status().is_redirection() {
            // 非 2xx 且非 3xx：返回给调用方判定（与旧行为一致：非 2xx 触发代理兜底）
            return Ok(resp);
        }
        // 3xx 重定向：提取 Location，做完整异步校验后跟随
        let location = resp
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| {
                Error::Other(format!("redirect without Location: {}", resp.status()))
            })?;
        // 相对/绝对解析：reqwest 的 Url::join 处理相对 Location
        let next_url = parsed_join(&current_url, location)?;
        warn!("Following redirect {} -> {}", current_url, next_url);
        current_url = next_url.to_string();
    }
    Err(Error::Other(format!(
        "too many redirects (>{}) for {}",
        MAX_REDIRECTS, start_url
    )))
}

/// 将可能相对的 Location 解析为绝对 URL（基于 base）。
fn parsed_join(base: &str, location: &str) -> Result<Url> {
    let base_url = Url::parse(base)
        .map_err(|e| Error::InvalidArgument(format!("invalid base URL: {}: {}", base, e)))?;
    base_url
        .join(location)
        .map_err(|e| Error::InvalidArgument(format!("invalid redirect Location '{}': {}", location, e)))
}

/// 应用自身 mihomo 混合端口代理地址（`http://127.0.0.1:{mixed_port}`）。
fn local_proxy_url(app: &AppHandle) -> String {
    let port = app
        .state::<crate::AppState>()
        .config_manager
        .lock()
        .unwrap()
        .get_config()
        .general
        .mixed_port;
    format!("http://127.0.0.1:{}", port)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SSRF：字面 IP 禁段（回环 / 私网 / 链路本地 / 未指定）一律拒绝。
    #[test]
    fn validate_url_rejects_blocked_literal_ips() {
        for url in [
            "http://127.0.0.1:8080/evil",
            "http://10.0.0.5/x",
            "http://172.16.0.1/x",
            "http://192.168.1.1/x",
            "http://169.254.169.254/latest/meta-data",
            "http://0.0.0.0/x",
            "http://[::1]:9090/x",
            "http://[fc00::1]/x",
            "http://[fe80::1]/x",
        ] {
            assert!(
                validate_url_sync(url).is_err(),
                "must reject blocked literal IP: {}",
                url
            );
        }
    }

    /// SSRF：非 http/https scheme 与 localhost/.local 主机名拒绝；合法公网 URL 放行。
    #[test]
    fn validate_url_rejects_bad_scheme_and_local_hosts() {
        for url in [
            "file:///etc/passwd",
            "ftp://example.com/x",
            "http://localhost:9090/x",
            "http://myhost.local/x",
            "http://sub.localhost/x",
        ] {
            assert!(
                validate_url_sync(url).is_err(),
                "must reject: {}",
                url
            );
        }
        for url in [
            "https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/geoip.dat",
            "http://www.gstatic.com/generate_204",
        ] {
            assert!(
                validate_url_sync(url).is_ok(),
                "must allow public URL: {}",
                url
            );
        }
    }

    /// SSRF：IPv4-mapped 地址（::ffff:a.b.c.d）必须按内嵌 V4 判定，防止绕过
    /// 回环/私网封锁（如 `http://[::ffff:127.0.0.1]:9090/` 直达本地控制器）。
    #[test]
    fn validate_url_rejects_ipv4_mapped_loopback_and_private() {
        for url in [
            "http://[::ffff:127.0.0.1]:9090/x",
            "http://[::ffff:127.0.0.2]/x",
            "http://[::ffff:10.0.0.5]/x",
            "http://[::ffff:192.168.1.1]/x",
            "http://[::ffff:169.254.169.254]/x",
        ] {
            assert!(
                validate_url_sync(url).is_err(),
                "must reject IPv4-mapped blocked address: {}",
                url
            );
        }
        assert!(
            validate_url_sync("http://[::ffff:8.8.8.8]/x").is_ok(),
            "IPv4-mapped public address must be allowed"
        );
    }

    /// 相对 Location 解析为绝对 URL
    #[test]
    fn parsed_join_resolves_relative_redirect() {
        let abs = parsed_join("https://example.com/a/b", "/c").unwrap();
        assert_eq!(abs.as_str(), "https://example.com/c");
        let abs = parsed_join("https://example.com/a/b", "c").unwrap();
        assert_eq!(abs.as_str(), "https://example.com/a/c");
        let abs = parsed_join("https://example.com/", "https://other.com/x").unwrap();
        assert_eq!(abs.as_str(), "https://other.com/x");
    }
}