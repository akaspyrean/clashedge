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

/// 统一 URL 脱敏（唯一实现，日志与前端展示共用）：只保留 scheme://host[:port]，
/// 删除 userinfo、query、fragment 与**完整 path**。
///
/// 为什么删 path：很多机场/订阅服务把 token 放在 path（如
/// `/api/v1/client/<token>`），而不仅是 query；保留 path 会把这类凭证原样泄露
/// 到日志/前端。有非根路径时以 `/…` 占位提示存在路径，避免误以为 URL 无路径。
/// 解析失败返回 "***"。
pub fn redact_url_for_log(url: &str) -> String {
    match Url::parse(url) {
        Ok(parsed) => {
            let mut out = String::new();
            out.push_str(parsed.scheme());
            out.push_str("://");
            if let Some(host) = parsed.host_str() {
                out.push_str(host);
            }
            if let Some(port) = parsed.port() {
                out.push(':');
                out.push_str(&port.to_string());
            }
            // path 可能携带 token，一律不保留；仅用占位标记提示原 URL 含路径。
            if !parsed.path().is_empty() && parsed.path() != "/" {
                out.push_str("/…");
            }
            out
        }
        Err(_) => "***".to_string(),
    }
}

use crate::util::error::{Error, Result};

/// 单次拉取超时（与既有订阅拉取行为一致）
const TIMEOUT: Duration = Duration::from_secs(30);

/// 最大重定向跳数（防重定向链被当作跳板无限转发/探测内网）
const MAX_REDIRECTS: usize = 3;

/// 拉取请求 User-Agent（部分订阅/下载服务器要求非空 UA）
/// 版本号取自 Cargo 包版本，避免发版后 UA 漂移
fn user_agent() -> String {
    format!("ClashEdge/{}", env!("CARGO_PKG_VERSION"))
}

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
    let parsed =
        Url::parse(url).map_err(|e| Error::InvalidArgument(format!("Invalid URL: {}", e)))?;
    let Some(host) = parsed.host_str() else {
        return Err(Error::InvalidArgument("URL has no host".to_string()));
    };
    let port = parsed
        .port()
        .unwrap_or(if parsed.scheme() == "https" { 443 } else { 80 });

    // 字面 IP 已在 sync 阶段校验；这里返回单元素列表供钉定
    if let Some(ip) = parse_host_ip(host) {
        return Ok(vec![SocketAddr::new(ip, port)]);
    }

    // 非 IP 主机名：DNS 解析后逐个校验
    let addrs = tokio::net::lookup_host((host, port)).await.map_err(|e| {
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
    let parsed =
        Url::parse(url).map_err(|e| Error::InvalidArgument(format!("Invalid URL: {}", e)))?;
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

/// 禁段 IP 判定：回环 / 私网 / 链路本地 / 未指定 / 广播 / 多播 / CGNAT。
/// 当前工具链的 `IpAddr` 上没有 `is_private`/`is_link_local`，按 V4/V6 分别判定；
/// IPv6 私网等价物为唯一本地地址（fc00::/7）、链路本地为 fe80::/10。
/// IPv4-mapped（::ffff:a.b.c.d）必须按内嵌 V4 判定，否则可经
/// `http://[::ffff:127.0.0.1]:9090/` 绕过回环/私网封锁。
fn is_denied_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_denied_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_denied_v4(v4);
            }
            v6.is_loopback()
                || v6.is_unique_local()
                || v6.is_unicast_link_local()
                || v6.is_unspecified()
                // IPv6 多播 ff00::/8
                || v6.is_multicast()
        }
    }
}

/// IPv4 禁段判定（含 IPv4-mapped 复用）：
/// 回环 / 私网 / 链路本地 / 未指定 / 受限广播 255.255.255.255 /
/// 多播 224.0.0.0/4 / CGNAT 100.64.0.0/10（RFC 6598，运营商级 NAT 段，
/// 不在 `is_private` 覆盖范围内，需单独判定）。
fn is_denied_v4(v4: std::net::Ipv4Addr) -> bool {
    let o = v4.octets();
    v4.is_loopback()
        || v4.is_private()
        || v4.is_link_local()
        || v4.is_unspecified()
        || v4.is_broadcast()
        || v4.is_multicast()
        || (o[0] == 100 && (o[1] & 0b1100_0000) == 0b0100_0000)
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
    get_direct_first_with_timeout(app, url, Some(TIMEOUT)).await
}

/// 大文件下载变体：`total_timeout=None` 时不设总超时（reqwest 的 `timeout()`
/// 覆盖整个请求周期含 body 读取，30s 会掐死几十 MB 的 geoip.dat 下载）。
/// 仅保留连接超时；调用方必须自行以「大小上限 + 整体 deadline」兜底，
/// 防止慢速/恶意源把下载任务无限挂起。
pub async fn get_direct_first_streaming(app: &AppHandle, url: &str) -> Result<reqwest::Response> {
    get_direct_first_with_timeout(app, url, None).await
}

/// P1-4：整条请求链（含全部重定向跳）保持同一路由语义。
/// 直连尝试的整个重定向链强制直连；代理兜底的整条链固定走本地代理。
/// 旧实现重定向跳重建 client 时未继承 no_proxy/proxy，会回落到
/// 系统代理/环境变量，导致同一链路前后两跳走不同网络路径。
#[derive(Clone, Copy, PartialEq, Eq)]
enum FetchRoute {
    Direct,
    LocalProxy,
}

/// 把路由语义应用到 client builder（首跳与每个重定向跳统一走这里）
fn apply_route(
    builder: reqwest::ClientBuilder,
    route: FetchRoute,
    proxy_url: &str,
) -> Result<reqwest::ClientBuilder> {
    Ok(match route {
        FetchRoute::Direct => builder.no_proxy(),
        FetchRoute::LocalProxy => builder.proxy(Proxy::all(proxy_url)?),
    })
}

async fn get_direct_first_with_timeout(
    app: &AppHandle,
    url: &str,
    total_timeout: Option<Duration>,
) -> Result<reqwest::Response> {
    // SSRF 防护：目标 URL 必须通过校验（含 DNS 禁段检查，返回已验地址）
    let resolved = validate_url(url).await?;

    // 1. 直连尝试（no_proxy：忽略系统代理/环境代理，强制直连）
    let direct = apply_route(
        build_client_with_resolved(url, &resolved, total_timeout)?,
        FetchRoute::Direct,
        "",
    )?
    .redirect(no_redirect_policy())
    .build()?;
    match send_and_follow(&direct, url, FetchRoute::Direct, "").await {
        Ok(resp) if resp.status().is_success() => return Ok(resp),
        Ok(resp) => {
            warn!(
                "Direct fetch got non-success status {} for {}; retrying via proxy",
                resp.status(),
                redact_url_for_log(url)
            );
        }
        Err(e) => {
            warn!(
                "Direct fetch failed for {}: {}; retrying via proxy",
                redact_url_for_log(url),
                e
            );
        }
    }

    // 2. 代理兜底：应用自身 mihomo 混合端口
    let proxy_url = local_proxy_url(app);
    info!(
        "Fetching {} via local proxy {}",
        redact_url_for_log(url),
        proxy_url
    );
    // P2：proxy 模式下 reqwest 的 resolve() 不生效（P1-5 确认）——proxy 收到
    // 原始域名后自己解析，留下 DNS rebinding TOCTOU 窗口。把 URL host 替换为
    // 已校验 IP 后（pin_url_host_to_ip），proxy 直接连 IP 不再解析域名，
    // 彻底关闭该窗口。
    let pinned_url = pin_url_host_to_ip(url, &resolved)?;
    let proxied = apply_route(
        build_client_with_resolved(&pinned_url, &resolved, total_timeout)?,
        FetchRoute::LocalProxy,
        &proxy_url,
    )?
    .redirect(no_redirect_policy())
    .build()?;
    send_and_follow(&proxied, &pinned_url, FetchRoute::LocalProxy, &proxy_url).await
}

/// 从 URL 提取主机名与已校验地址，构建带 `resolve()` 钉定的 ClientBuilder。
/// 钉定后 reqwest 连接时直接用已校验的 IP，不再重新解析 DNS（关闭 TOCTOU）。
fn build_client_with_resolved(
    url: &str,
    resolved: &[SocketAddr],
    total_timeout: Option<Duration>,
) -> Result<reqwest::ClientBuilder> {
    let parsed =
        Url::parse(url).map_err(|e| Error::InvalidArgument(format!("Invalid URL: {}", e)))?;
    let mut builder = reqwest::Client::builder();
    if let Some(t) = total_timeout {
        builder = builder.timeout(t);
    }
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

/// P2：把 URL 的 host 替换为已校验 IP 列表的第一个地址。
///
/// P1-5 已确认：reqwest 的 `resolve()` 在 `.proxy(Proxy::all(..))` 模式下
/// **不生效**——proxy（mihomo）收到的是原始域名，随后自己解析。这留下
/// DNS rebinding TOCTOU 窗口：`validate_url` 校验时域名是公网 IP（通过），
/// 直连失败后走 local proxy fallback，proxy 重新解析时域名已被攻击者改为
/// 127.0.0.1 → mihomo 连回本机（SSRF）。
///
/// 修复：把 URL host 替换成已校验 IP 后，proxy 收到的是 IP 而非域名，
/// 直接连 IP，不再解析域名，彻底关闭 DNS rebinding 窗口。
///
/// 权衡：HTTPS 下 reqwest 用 IP 做 SNI，严格校验 SNI 的服务器可能拒绝。
/// 这是「安全（SSRF 防护）优先于兼容性」的取舍；订阅/geodata 下载目标
/// 多为 CDN，通常不严格校验 SNI。
///
/// `resolved` 为空时原样返回（不改写）。
fn pin_url_host_to_ip(url: &str, resolved: &[SocketAddr]) -> Result<String> {
    let Some(first) = resolved.first() else {
        return Ok(url.to_string());
    };
    let ip = first.ip();
    // IPv6 host 需方括号形式；IPv4 直接字符串
    let host = match ip {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => format!("[{}]", v6),
    };
    let port = first.port();
    let mut parsed =
        Url::parse(url).map_err(|e| Error::InvalidArgument(format!("Invalid URL: {}", e)))?;
    parsed
        .set_host(Some(&host))
        .map_err(|e| Error::Other(format!("Failed to pin URL host to IP: {}", e)))?;
    // set_port 错误类型为 ()，无 _err 可格式化
    parsed
        .set_port(Some(port))
        .map_err(|()| Error::Other("Failed to pin URL port".to_string()))?;
    Ok(parsed.to_string())
}

/// 发送请求并手动处理重定向：每跳做完整异步 SSRF 校验（含 DNS）后
/// 用钉定连接跟随，避免自动重定向的同步回调无法做 DNS 校验。
/// P1-4：重定向跳重建 client 时通过 `apply_route` 保持与首跳相同的
/// 直连/代理路由语义，整条链路网络路径一致。
async fn send_and_follow(
    client: &reqwest::Client,
    start_url: &str,
    route: FetchRoute,
    proxy_url: &str,
) -> Result<reqwest::Response> {
    let mut current_url = start_url.to_string();
    for _hop in 0..=MAX_REDIRECTS {
        // 每跳（含首跳）都校验：首跳已由调用方校验并（proxy 下）pin 好，
        // 重定向到的新 URL 需要新校验。
        let resolved = if _hop == 0 {
            Vec::new()
        } else {
            validate_url(&current_url).await?
        };
        // P2：proxy 模式下 resolve() 不生效，重定向到的新主机也必须 pin 成
        // 已校验 IP——否则 proxy 对重定向目标自己解析，仍存在 DNS rebinding
        // 窗口。首跳 start_url 已由调用方 pin 好（get_direct_first proxy 路径）。
        let req_url = if route == FetchRoute::LocalProxy && !resolved.is_empty() {
            pin_url_host_to_ip(&current_url, &resolved)?
        } else {
            current_url.clone()
        };
        // 对于重定向跳，需要用新 URL 的已验地址重新构建请求（req_url 已 pin）；
        // 但 client 是共享的（已钉定首跳主机）。重定向到新主机名时，
        // 我们用独立钉定 client 发送。为关闭 TOCTOU，重定向到新主机时
        // 改用独立钉定 client。
        let req_client = if _hop == 0 {
            // 首跳：client 已由调用方钉定
            client.clone()
        } else if resolved.is_empty() {
            client.clone()
        } else {
            apply_route(
                build_client_with_resolved(&req_url, &resolved, None)?,
                route,
                proxy_url,
            )?
            .redirect(no_redirect_policy())
            .build()?
        };

        let resp = req_client
            .get(&req_url)
            .header(USER_AGENT, user_agent())
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
            .ok_or_else(|| Error::Other(format!("redirect without Location: {}", resp.status())))?;
        // 相对/绝对解析：reqwest 的 Url::join 处理相对 Location
        let next_url = parsed_join(&current_url, location)?;
        warn!(
            "Following redirect {} -> {}",
            redact_url_for_log(&current_url),
            redact_url_for_log(next_url.as_str())
        );
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
    base_url.join(location).map_err(|e| {
        Error::InvalidArgument(format!("invalid redirect Location '{}': {}", location, e))
    })
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
            assert!(validate_url_sync(url).is_err(), "must reject: {}", url);
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

    /// SSRF：CGNAT 100.64.0.0/10、广播 255.255.255.255、IPv4 多播 224.0.0.0/4
    /// 一律拒绝；段边界外的相邻地址必须放行（防误伤公网）。
    #[test]
    fn validate_url_rejects_cgnat_broadcast_and_multicast() {
        for url in [
            // CGNAT 段内 + 两个边界
            "http://100.64.0.0/x",
            "http://100.64.0.1/x",
            "http://100.127.255.254/x",
            "http://100.127.255.255/x",
            // IPv4 受限广播
            "http://255.255.255.255/x",
            // 多播段边界：224.0.0.0（下界）与 239.255.255.255（上界）
            "http://224.0.0.0/x",
            "http://224.0.0.1/x",
            "http://239.255.255.255/x",
            // IPv4-mapped 形式的多播同样拒绝
            "http://[::ffff:224.0.0.1]/x",
            "http://[::ffff:100.64.0.1]/x",
        ] {
            assert!(
                validate_url_sync(url).is_err(),
                "must reject blocked address: {}",
                url
            );
        }
        for url in [
            // CGNAT 下界前一个 / 上界后一个：属公网，必须放行
            "http://100.63.255.255/x",
            "http://100.128.0.0/x",
            // 多播上界之后（223.x 为公网）与 240.x（保留但非多播/非本应用禁段）
            "http://223.255.255.255/x",
            "http://8.8.8.8/x",
        ] {
            assert!(
                validate_url_sync(url).is_ok(),
                "must allow public address: {}",
                url
            );
        }
    }

    /// SSRF：IPv6 多播 ff00::/8 一律拒绝；边界外单播放行。
    #[test]
    fn validate_url_rejects_ipv6_multicast() {
        for url in [
            "http://[ff00::1]/x",
            "http://[ff01::1]/x",
            "http://[ff02::1]/x",                                 // 所有节点多播
            "http://[ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff]/x", // ff 上界附近
        ] {
            assert!(
                validate_url_sync(url).is_err(),
                "must reject IPv6 multicast: {}",
                url
            );
        }
        assert!(
            validate_url_sync("http://[fe80::1]/x").is_err(),
            "link-local must still be rejected"
        );
        assert!(
            validate_url_sync("http://[2606:4700:4700::1111]/x").is_ok(),
            "public IPv6 unicast must be allowed"
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

    // --- P1-5/P2：SSRF local-proxy fallback 修复验证 -------------------------
    //
    // 审计担忧：reqwest 的 `resolve()`（DNS pinning）在 `.proxy(Proxy::all(...))`
    // 模式下不生效——proxy 收到的是原始域名而非已校验 IP。这意味着：
    //   1. validate_url 校验 evil.com → 返回公网 IP（通过）
    //   2. direct 失败 → 触发 local proxy fallback
    //   3. proxy（mihomo）重新解析 evil.com → DNS rebinding 返回 127.0.0.1
    //   4. mihomo 连 127.0.0.1（SSRF 成功）
    //
    // P2 修复：`pin_url_host_to_ip` 把 URL host 替换为已校验 IP，proxy 收到
    // IP 而非域名，直接连 IP、不再解析域名，彻底关闭 DNS rebinding 窗口。
    // 下面两个测试验证修复生效：proxy 收到的是 IP，而非域名。

    /// 单元测试：pin_url_host_to_ip 把域名 host 替换为已校验 IP（IPv4），
    /// 保留路径/query/port。
    #[test]
    fn pin_url_host_to_ip_replaces_domain_with_ip_v4() {
        let resolved = vec![SocketAddr::new(
            std::net::Ipv4Addr::new(203, 0, 113, 1).into(),
            8080,
        )];
        let pinned =
            pin_url_host_to_ip("http://evil.example:8080/path?token=secret", &resolved).unwrap();
        assert_eq!(pinned, "http://203.0.113.1:8080/path?token=secret");
    }

    /// 单元测试：pin_url_host_to_ip 处理 IPv6（方括号形式）。
    #[test]
    fn pin_url_host_to_ip_replaces_domain_with_ip_v6() {
        let resolved = vec![SocketAddr::new(
            "2606:4700:4700::1111"
                .parse::<std::net::Ipv6Addr>()
                .unwrap()
                .into(),
            443,
        )];
        let pinned = pin_url_host_to_ip("https://evil6.example/x", &resolved).unwrap();
        // url crate 对 scheme 默认端口（https=443）不显式输出，符合预期
        assert_eq!(pinned, "https://[2606:4700:4700::1111]/x");
    }

    /// 单元测试：resolved 为空时原样返回（不改写）。
    #[test]
    fn pin_url_host_to_ip_returns_original_when_empty() {
        let pinned = pin_url_host_to_ip("https://example.com/x", &[]).unwrap();
        assert_eq!(pinned, "https://example.com/x");
    }

    /// integration：proxy 模式下，发送的请求必须带已校验 IP（而非域名），
    /// 证明 DNS rebinding 窗口已关闭。
    #[tokio::test]
    async fn proxy_mode_sends_pinned_ip_not_domain() {
        use std::sync::Arc;
        use tokio::sync::Mutex;

        // 起一个本地 HTTP proxy，记录收到的请求
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_port = listener.local_addr().unwrap().port();
        let received_req: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let req_clone = received_req.clone();

        let proxy_task = tokio::spawn(async move {
            let (mut conn, _) = listener.accept().await.unwrap();
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut buf = [0u8; 1024];
            let _ = conn.read(&mut buf).await;
            let req_text = String::from_utf8_lossy(&buf).to_string();
            *req_clone.lock().await = Some(req_text);
            let _ = conn
                .write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n")
                .await;
        });

        let url = "http://myhost.test:80/";
        let resolved = vec![SocketAddr::new(
            std::net::Ipv4Addr::new(203, 0, 113, 1).into(),
            80,
        )];
        // 与 get_direct_first_with_timeout proxy 路径一致：先 pin URL。
        let pinned_url = pin_url_host_to_ip(url, &resolved).unwrap();
        let builder =
            build_client_with_resolved(&pinned_url, &resolved, Some(Duration::from_secs(5)))
                .unwrap();
        let proxy_url = format!("http://127.0.0.1:{}", proxy_port);
        let client = apply_route(builder, FetchRoute::LocalProxy, &proxy_url)
            .unwrap()
            .redirect(no_redirect_policy())
            .build()
            .unwrap();

        let _ = client
            .get(&pinned_url)
            .header(USER_AGENT, user_agent())
            .send()
            .await;

        let _ = tokio::time::timeout(Duration::from_secs(2), proxy_task).await;

        let seen = received_req.lock().await.clone();
        assert!(seen.is_some(), "proxy should have received a request");
        let req_text = seen.unwrap();
        let first_line = req_text.lines().next().unwrap_or("");
        // P2 修复断言：请求目标必须是 IP（203.0.113.1），而非域名 myhost.test。
        assert!(
            first_line.contains("203.0.113.1"),
            "proxy request target must be the pinned IP, got: {}",
            first_line
        );
        assert!(
            !first_line.contains("myhost.test"),
            "proxy request target must NOT be the original domain, got: {}",
            first_line
        );
    }
}
