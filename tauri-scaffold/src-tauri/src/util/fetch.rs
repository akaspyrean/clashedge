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
//! 安全（C2 SSRF 防护）：所有拉取目标 URL 必须通过 `validate_url` 校验
//! （拒绝非 http/https、localhost/.local、回环/私网/链路本地等禁段），
//! 并对直连与代理兜底两个 client 统一施加自定义重定向策略——跳转前逐跳
//! 校验，命中禁段即中止，且限制最大跳数。校验的是**目标 URL**，不是代理地址。

use std::net::IpAddr;
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
/// - 非 IP 主机名解析出的**所有**地址均落在禁段（解析失败保守放行并 warn）。
pub async fn validate_url(url: &str) -> Result<()> {
    validate_url_sync(url)?;
    let parsed = Url::parse(url)
        .map_err(|e| Error::InvalidArgument(format!("Invalid URL: {}", e)))?;
    // 字面 IP 已在 sync 阶段校验；这里只补非 IP 主机名的 DNS 禁段检查
    if let Some(host) = parsed.host_str() {
        if parse_host_ip(host).is_none() {
            let port = parsed.port().unwrap_or(80);
            match tokio::net::lookup_host((host, port)).await {
                Ok(addrs) => {
                    let addrs: Vec<std::net::SocketAddr> = addrs.collect();
                    if !addrs.is_empty() && addrs.iter().all(|a| is_denied_ip(a.ip())) {
                        return Err(Error::InvalidArgument(format!(
                            "URL host '{}' resolves only to blocked addresses",
                            host
                        )));
                    }
                }
                Err(e) => {
                    // 解析失败（DNS 临时故障等）保守放行，避免误伤正常订阅源
                    warn!(
                        "URL host '{}' DNS lookup failed ({}); allowing (conservative)",
                        host, e
                    );
                }
            }
        }
    }
    Ok(())
}

/// 同步 URL 校验（scheme / 主机名 / 字面 IP）。
/// 供 `get_direct_first` 与重定向策略在跳转前逐跳调用（策略回调是同步的，
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
                return v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_unspecified();
            }
            v6.is_loopback()
                || v6.is_unique_local()
                || v6.is_unicast_link_local()
                || v6.is_unspecified()
        }
    }
}

/// 自定义重定向策略：跳转前对每个目标 URL 做 SSRF 校验，命中禁段即中止；
/// 并限制最大跳数，防止被重定向链当作跳板探测内网。
fn redirect_policy() -> Policy {
    Policy::custom(move |attempt| {
        if attempt.previous().len() >= MAX_REDIRECTS {
            return attempt.error("too many redirects");
        }
        match validate_url_sync(attempt.url().as_str()) {
            Ok(()) => attempt.follow(),
            Err(e) => attempt.error(e.to_string()),
        }
    })
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
/// SSRF：目标 URL 必须先通过 `validate_url`；直连与代理兜底访问同一目标，
/// 校验的是目标而非代理地址。
pub async fn get_direct_first(app: &AppHandle, url: &str) -> Result<reqwest::Response> {
    // C2 SSRF 防护：目标 URL 必须通过校验（含 DNS 禁段检查）
    validate_url(url).await?;

    // 1. 直连尝试（no_proxy：忽略系统代理/环境代理，强制直连）
    let direct = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .no_proxy()
        .redirect(redirect_policy())
        .build()?;
    match direct
        .get(url)
        .header(USER_AGENT, USER_AGENT_VALUE)
        .send()
        .await
    {
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
    let proxied = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .proxy(Proxy::all(&proxy_url)?)
        .redirect(redirect_policy())
        .build()?;
    proxied
        .get(url)
        .header(USER_AGENT, USER_AGENT_VALUE)
        .send()
        .await
        .map_err(Error::from)
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

    /// C2：字面 IP 禁段（回环 / 私网 / 链路本地 / 未指定）一律拒绝。
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

    /// C2：非 http/https scheme 与 localhost/.local 主机名拒绝；合法公网 URL 放行。
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

    /// C2：IPv4-mapped 地址（::ffff:a.b.c.d）必须按内嵌 V4 判定，防止绕过
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
}