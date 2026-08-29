// src-tauri/src/util/fetch.rs
//! 拉取助手：直连优先、直连不通自动切换到应用自身代理兜底
//!
//! 需求：订阅更新、配置更新（geodata）这类"拉取动作"默认走直连；
//! 直连不通时自动改走应用自身 mihomo 混合端口（`127.0.0.1:{mixed_port}`）
//! 的 SOCKS5 本地解析模式重试。软件本身的代理模式不受影响。
//!
//! 判定"直连不通"：直连请求连接失败/超时，或返回非 2xx 状态码
//! （服务器对直连 IP 限流/屏蔽时常见，走代理可绕过）。
//!
//! 安全（SSRF 防护）：所有拉取目标 URL 必须通过 `validate_url` 校验——
//! 采用**严格白名单**语义：scheme 限定 http/https、拒绝 localhost/.local
//! 主机名，且字面 IP 与 DNS 解析结果必须**全部**是"全球可路由公网地址"
//! 才放行。回环/私网/链路本地/CGNAT/TEST-NET/基准测试/多播/保留段等
//! 非全球地址一律拒绝（白名单比黑名单更稳：IANA 新增保留段时默认拒绝）。
//! 另对直连与代理兜底两个 client 统一施加手动重定向处理——跳转前逐跳
//! 做完整异步校验（含 DNS 检查），命中非全球地址即中止，且限制最大跳数。
//! 校验的是**目标 URL**，不是代理地址。
//!
//! 超时语义：整条重定向链共享一个总 deadline（`Instant`），每一跳都用
//! 剩余时间约束（client 总超时 + 每跳 `tokio::time::timeout` 双保险），
//! 并统一施加连接超时与读取超时（streaming 模式仍不设总超时，靠调用方
//! 兜底，见 `get_direct_first_streaming`）。
//!
//! TOCTOU 收敛：`validate_url` 解析 DNS 后返回已校验的地址列表，
//! `get_direct_first` 用 `reqwest::Client::resolve()` 将主机名钉定到
//! 已校验的 IP，避免"校验一次 DNS、连接时重新解析"的窗口。重定向
//! 到新主机名时同样先异步校验再钉定连接。

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::{Duration, Instant};

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

/// 所有 client（直连/代理、首跳/重定向跳）统一的 TCP/TLS 连接超时。
/// 连接阶段不受重定向链总 deadline 的"逐跳剩余时间"精确划分影响，
/// 单独给一个硬上限防止握手挂死。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// 单次读取超时：只约束**一次 read 间隔**（响应头读取、body 相邻 chunk 间隔），
/// 不限制总时长。streaming 模式（无总超时）下载大文件时，只要每次 read 有
/// 进展就不会被掐死；慢速/恶意源的整体卡死仍需调用方以「大小上限 + 整体
/// deadline」兜底。
const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// 每跳最少剩余时间：重定向链共享总 deadline 时，剩余时间低于该值不再发起新跳，
/// 直接判总超时（避免发起注定失败的连接，也覆盖 DNS 校验阶段耗时）。
const MIN_HOP_REMAINING: Duration = Duration::from_millis(100);

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
/// - 字面 IP 不是全球可路由公网地址（严格白名单，见 `is_globally_routable`）；
/// - 非 IP 主机名解析出的地址中**只要存在**任一非全球地址即拒绝
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
    let mut addrs: Vec<SocketAddr> = addrs.collect();
    if addrs.is_empty() {
        return Err(Error::InvalidArgument(format!(
            "URL host '{}' resolved to no addresses; rejected",
            host
        )));
    }
    // DNS rebinding / 混合解析防护：只要存在任一非全球可路由地址即整体拒绝
    ensure_all_globally_routable(host, &addrs)?;
    // Windows/代理链对 IPv4 的可用性通常更稳定；优先尝试 IPv4，但保留全部
    // 已验证地址供连接层故障转移。排序不改变 SSRF 结论（所有地址已逐一通过）。
    addrs.sort_by_key(|addr| addr.is_ipv6());
    addrs.dedup();
    Ok(addrs)
}

/// DNS 校验循环（独立成函数便于单测）：解析结果中只要存在任一非全球可路由
/// 地址即整体拒绝（DNS rebinding / 混合解析防护），纯公网结果放行。
fn ensure_all_globally_routable(host: &str, addrs: &[SocketAddr]) -> Result<()> {
    for a in addrs {
        if !is_globally_routable(a.ip()) {
            return Err(Error::InvalidArgument(format!(
                "URL host '{}' resolves to a non-globally-routable address ({}); rejected",
                host,
                a.ip()
            )));
        }
    }
    Ok(())
}

/// 同步 URL 校验（scheme / 主机名 / 字面 IP 白名单）。
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
        if !is_globally_routable(ip) {
            return Err(Error::InvalidArgument(format!(
                "URL host '{}' is not a globally routable public address",
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

/// 全球可路由公网地址白名单：**地址必须是"全球可路由公网地址"才允许**。
/// 语义与 IANA IPv4/IPv6 Special-Purpose Address Registry 中
/// "Globally Reachable = False" 的段一致，即与 std `Ipv4Addr::is_global` /
/// `Ipv6Addr::is_global` 的判定语义对齐。
///
/// 注：本机 stable 工具链（1.97.1）上 `is_global()` 仍属 nightly-only 的
/// `ip` feature（#27709，经快速编译验证报 E0658），因此这里按 std 源码
/// 同款逻辑手写实现；后续 std 稳定后可切换为 `ip.is_global()`。
/// IPv4-mapped（::ffff:a.b.c.d）必须按内嵌 V4 判定，否则可经
/// `http://[::ffff:127.0.0.1]:9090/` 绕过。
fn is_globally_routable(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_globally_routable_v4(v4),
        IpAddr::V6(v6) => is_globally_routable_v6(v6),
    }
}

/// IPv4 白名单判定（含 IPv4-mapped 复用）。与 std `is_global` 实现同款：
/// 192.0.0.0/24（IETF 协议指派段）整体不可全球路由，但 192.0.0.9（PCP）
/// 与 192.0.0.10（NAT64 well-known）在 IANA 注册表标记为全球可达，按
/// std 语义放行；其余命中任何非全球段即拒绝。
fn is_globally_routable_v4(v4: Ipv4Addr) -> bool {
    let [o0, o1, o2, o3] = v4.octets();
    // 0.0.0.0/8（含 0.0.0.0 未指定与 0.x "this network" 段）
    // 10.0.0.0/8、172.16.0.0/12、192.168.0.0/16 私网（RFC 1918）
    // 127.0.0.0/8 回环
    // 169.254.0.0/16 链路本地
    // 100.64.0.0/10 CGNAT/共享地址空间（RFC 6598）
    // 192.0.2.0/24、198.51.100.0/24、203.0.113.0/24 文档段（RFC 5737）
    // 192.88.99.0/24 已弃用的 6to4 中继任播
    // 198.18.0.0/15 基准测试段（RFC 2544）
    // 224.0.0.0/4 多播
    // 240.0.0.0/4 保留段（含 255.255.255.255 受限广播）
    if o0 == 192 && o1 == 0 && o2 == 0 {
        // 192.0.0.0/24：仅 192.0.0.9/192.0.0.10 全球可达（std is_global 同款例外）
        return o3 == 9 || o3 == 10;
    }
    o0 != 0
        && !v4.is_private()
        && !v4.is_loopback()
        && !v4.is_link_local()
        && !v4.is_broadcast()
        && !v4.is_documentation()
        && !(o0 == 100 && (o1 & 0b1100_0000) == 0b0100_0000)
        && !(o0 == 192 && o1 == 88 && o2 == 99)
        && !(o0 == 198 && (o1 & 0b1111_1110) == 18)
        && !((o0 & 0b1111_0000) == 0b1110_0000)
        && !((o0 & 0b1111_0000) == 0b1111_0000)
}

/// IPv6 白名单判定：
/// - `::/96`（未指定 ::、回环 ::1、已废弃的 IPv4-compatible 形式 ::a.b.c.d，
///   如 `::10.0.0.5`）整段拒绝——防止 <32 位前缀形式绕过 IPv4 检查；
/// - IPv4-mapped（::ffff:a.b.c.d）解包按 V4 白名单判定；
/// - 其余按 IANA 语义拒绝 ULA/链路本地/多播/文档段/NAT64/discard 段及
///   已弃用的 Teredo/6to4，仅放行全球单播。
fn is_globally_routable_v6(v6: Ipv6Addr) -> bool {
    let seg = v6.segments();
    // ::/96：前 6 组为 0（含 :: 与 ::1）；IPv4-mapped 的第 6 组是 0xffff，不受影响
    if seg[0] == 0 && seg[1] == 0 && seg[2] == 0 && seg[3] == 0 && seg[4] == 0 && seg[5] == 0 {
        return false;
    }
    if let Some(v4) = v6.to_ipv4_mapped() {
        return is_globally_routable_v4(v4);
    }
    !v6.is_multicast()
        && !v6.is_loopback()
        && !v6.is_unspecified()
        && !v6.is_unique_local()
        && !v6.is_unicast_link_local()
        // 64:ff9b::/96 NAT64 well-known prefix（IANA：不可全球路由）
        && !(seg[0] == 0x0064 && seg[1] == 0xff9b)
        // 100::/64 discard-only（IANA：不可全球路由）
        && seg[0] != 0x0100
        // 2001:db8::/32 文档段
        && !(seg[0] == 0x2001 && seg[1] == 0x0db8)
        // 2001::/32 Teredo（已弃用，std is_global 语义：非全球）
        && !(seg[0] == 0x2001 && seg[1] == 0x0000)
        // 2002::/16 6to4（已弃用，std is_global 语义：非全球）
        && seg[0] != 0x2002
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
///    （`socks5://127.0.0.1:{mixed_port}`）作本地解析的 SOCKS5 代理重试一次；
/// 3. 返回最终 response（调用方负责消费 body / 检查状态码）。
///
/// 返回的 response 状态码不保证是 2xx——调用方需自行判定；
/// 但若直连已拿到 2xx，则不会发起代理重试。
///
/// SSRF + TOCTOU：目标 URL 必须先通过 `validate_url`（含 DNS 白名单检查，
/// 解析失败即拒绝）；校验返回的已验地址用 `resolve()` 钉定到客户端，
/// 避免连接时重新解析。重定向到新主机名时逐跳做完整异步校验再钉定。
pub async fn get_direct_first(app: &AppHandle, url: &str) -> Result<reqwest::Response> {
    get_direct_first_with_timeout(app, url, Some(TIMEOUT)).await
}

/// 大文件下载变体：`total_timeout=None` 时不设总超时（reqwest 的 `timeout()`
/// 覆盖整个请求周期含 body 读取，30s 会掐死几十 MB 的 geoip.dat 下载）。
/// 仅保留连接超时与读取超时（读取超时只约束单次 read 间隔，不限制总时长）；
/// 调用方必须自行以「大小上限 + 整体 deadline」兜底，防止慢速/恶意源把
/// 下载任务无限挂起。
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
    // SSRF 防护：目标 URL 必须通过校验（含 DNS 白名单检查，返回已验地址）
    let resolved = validate_url(url).await?;

    // 1. 直连尝试（no_proxy：忽略系统代理/环境代理，强制直连）
    let direct = apply_route(
        build_client_with_resolved(url, &resolved, total_timeout)?,
        FetchRoute::Direct,
        "",
    )?
    .redirect(no_redirect_policy())
    .build()?;
    // 2. 代理兜底：应用自身 mihomo 混合端口
    let proxy_url = local_proxy_url(app);
    let proxied = apply_route(
        build_client_with_resolved(url, &resolved, total_timeout)?,
        FetchRoute::LocalProxy,
        &proxy_url,
    )?
    .redirect(no_redirect_policy())
    .build()?;
    // 总超时一路传递：direct 与 proxied 两条链各自持有同一总预算
    send_direct_then_proxy(&direct, &proxied, url, &proxy_url, total_timeout).await
}

async fn send_direct_then_proxy(
    direct: &reqwest::Client,
    proxied: &reqwest::Client,
    url: &str,
    proxy_url: &str,
    total_timeout: Option<Duration>,
) -> Result<reqwest::Response> {
    match send_and_follow(direct, url, FetchRoute::Direct, "", total_timeout).await {
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

    info!(
        "Fetching {} via local proxy {}",
        redact_url_for_log(url),
        proxy_url
    );
    // HTTP CONNECT 代理会收到原始 hostname 并自行解析；而把 HTTPS URL 改成 IP
    // 又会破坏 TLS SNI/证书 hostname 校验。mixed-port 同时支持 SOCKS5：使用
    // `socks5://`（本地解析，不是 socks5h）+ reqwest resolve pinning，SOCKS 请求
    // 只携带已验证 IP，同时 URL/Host/SNI 始终保留原 hostname。
    send_and_follow(
        proxied,
        url,
        FetchRoute::LocalProxy,
        proxy_url,
        total_timeout,
    )
    .await
}

/// 从 URL 提取主机名与已校验地址，构建带 `resolve()` 钉定的 ClientBuilder。
/// 钉定后 reqwest 连接时直接用已校验的 IP，不再重新解析 DNS（关闭 TOCTOU）。
/// 统一施加连接超时与读取超时；`total_timeout` 为该 client 的总超时
/// （重定向跳传剩余时间；streaming 模式传 None，此时仅靠 connect/read
/// 超时与调用方兜底）。
fn build_client_with_resolved(
    url: &str,
    resolved: &[SocketAddr],
    total_timeout: Option<Duration>,
) -> Result<reqwest::ClientBuilder> {
    let parsed =
        Url::parse(url).map_err(|e| Error::InvalidArgument(format!("Invalid URL: {}", e)))?;
    let mut builder = reqwest::Client::builder()
        // 连接阶段硬上限：TCP/TLS 握手挂死不受总超时逐跳划分的精确度影响
        .connect_timeout(CONNECT_TIMEOUT)
        // 读取超时只约束单次 read 间隔（响应头、body chunk 间隔），不限制总时长：
        // streaming 模式依然可慢慢下载大文件，只要每次 read 有进展。
        .read_timeout(READ_TIMEOUT);
    if let Some(t) = total_timeout {
        builder = builder.timeout(t);
    }
    if let Some(host) = parsed.host_str() {
        if !resolved.is_empty() {
            let host_no_brackets = host
                .strip_prefix('[')
                .and_then(|h| h.strip_suffix(']'))
                .unwrap_or(host);
            builder = builder.resolve_to_addrs(host_no_brackets, resolved);
        }
    }
    Ok(builder)
}

/// 发送请求并手动处理重定向：每跳做完整异步 SSRF 校验（含 DNS）后
/// 用钉定连接跟随，避免自动重定向的同步回调无法做 DNS 校验。
/// P1-4：重定向跳重建 client 时通过 `apply_route` 保持与首跳相同的
/// 直连/代理路由语义，整条链路网络路径一致。
///
/// 重定向链总 deadline：`total_timeout` 在入口换算为 `Instant` deadline，
/// 整条链（首跳 + 全部重定向跳）共享。每一跳：
/// 1. 剩余时间低于 `MIN_HOP_REMAINING` 直接判总超时（不再发起新跳）；
/// 2. 重定向跳重建 client 时以剩余时间作为该 client 的总超时
///    （`None` 总超时的 streaming 模式除外）；
/// 3. 发送动作用 `tokio::time::timeout(remaining, …)` 双保险包裹——
///    即使 client 总超时在 DNS/连接阶段划分不精确，也不会超过总 deadline。
async fn send_and_follow(
    client: &reqwest::Client,
    start_url: &str,
    route: FetchRoute,
    proxy_url: &str,
    total_timeout: Option<Duration>,
) -> Result<reqwest::Response> {
    let deadline = total_timeout.map(|t| Instant::now() + t);
    let mut current_url = start_url.to_string();
    for _hop in 0..=MAX_REDIRECTS {
        // 每跳剩余时间：deadline 已耗尽（或不足最小阈值）直接判总超时
        let remaining = deadline.map(|d| d.saturating_duration_since(Instant::now()));
        if let Some(r) = remaining {
            if r < MIN_HOP_REMAINING {
                return Err(Error::Other(format!(
                    "request timed out: total deadline nearly exhausted before hop {} \
                     (remaining {:?} < minimum {:?}) for {}",
                    _hop,
                    r,
                    MIN_HOP_REMAINING,
                    redact_url_for_log(&current_url)
                )));
            }
        }
        // 每跳（含首跳）都校验：首跳已由调用方校验并通过 resolve pin 好，
        // 重定向到的新 URL 需要新校验。
        let resolved = if _hop == 0 {
            Vec::new()
        } else {
            validate_url(&current_url).await?
        };
        let req_url = current_url.clone();
        // 对于重定向跳，需要用新 URL 的已验地址重新构建请求；
        // 但 client 是共享的（已钉定首跳主机）。重定向到新主机名时，
        // 我们用独立钉定 client 发送。为关闭 TOCTOU，重定向到新主机时
        // 改用独立钉定 client。
        let req_client = if _hop == 0 {
            // 首跳：client 已由调用方钉定
            client.clone()
        } else if resolved.is_empty() {
            client.clone()
        } else {
            // 重定向跳：原 client 的总超时只覆盖首跳，这里以剩余时间重建，
            // 保证整条链共享同一总 deadline（None 总超时的 streaming 模式除外）
            apply_route(
                build_client_with_resolved(&req_url, &resolved, remaining)?,
                route,
                proxy_url,
            )?
            .redirect(no_redirect_policy())
            .build()?
        };

        let send_fut = req_client
            .get(&req_url)
            .header(USER_AGENT, user_agent())
            .send();
        // 双保险：每跳发送都受剩余时间硬约束（覆盖 DNS/连接阶段与 client
        // 总超时划分不精确的窗口）
        let resp = if let Some(r) = remaining {
            match tokio::time::timeout(r, send_fut).await {
                Ok(inner) => inner?,
                Err(_elapsed) => {
                    return Err(Error::Other(format!(
                        "request timed out: hop {} exceeded remaining {:?} of the shared \
                         total deadline for {}",
                        _hop,
                        r,
                        redact_url_for_log(&req_url)
                    )))
                }
            }
        } else {
            send_fut.await?
        };

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

/// 应用自身 mihomo mixed-port 的 SOCKS5 地址。`socks5` 表示客户端解析目标域名；
/// 禁止改成 `socks5h`，后者会让代理端重新 DNS 解析并重开 rebinding 窗口。
fn local_proxy_url(app: &AppHandle) -> String {
    let port = app
        .state::<crate::AppState>()
        .config_manager
        .lock()
        .unwrap()
        .get_config()
        .general
        .mixed_port;
    format!("socks5://127.0.0.1:{}", port)
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

    /// SSRF 白名单：0.0.0.0/8、基准测试 198.18.0.0/15、保留 240.0.0.0/4、
    /// 广播、TEST-NET 文档段、192.0.0.0/24（除 .9/.10）等非全球可路由地址
    /// 一律拒绝（旧黑名单放行了其中多数）。
    #[test]
    fn validate_url_rejects_non_globally_routable_literals() {
        for url in [
            // 0.0.0.0/8
            "http://0.1.2.3/x",
            "http://0.0.0.0/x",
            // 198.18.0.0/15 两个端点
            "http://198.18.0.1/x",
            "http://198.19.255.255/x",
            // 240.0.0.0/4
            "http://240.0.0.1/x",
            // 受限广播
            "http://255.255.255.255/x",
            // RFC 5737 文档段（TEST-NET）
            "http://192.0.2.1/x",
            "http://198.51.100.7/x",
            "http://203.0.113.9/x",
            // 192.0.0.0/24（IETF 协议指派段，除 .9/.10）
            "http://192.0.0.1/x",
            // IPv4-mapped 形式的非全球地址同样按内嵌 V4 拒绝
            "http://[::ffff:0.1.2.3]/x",
            "http://[::ffff:198.18.0.1]/x",
            "http://[::ffff:240.0.0.1]/x",
            "http://[::ffff:203.0.113.9]/x",
        ] {
            assert!(
                validate_url_sync(url).is_err(),
                "must reject non-globally-routable literal: {}",
                url
            );
        }
    }

    /// SSRF 白名单防误伤：典型公网地址（含 CGNAT 边界外、多播上界外）
    /// 必须放行；192.0.0.9（PCP）/192.0.0.10（NAT64 well-known）在 IANA
    /// 注册表标记全球可达，按 std is_global 同款例外语义放行（见
    /// `is_globally_routable_v4` 注释）。
    #[test]
    fn validate_url_allows_public_literals() {
        for url in [
            "http://8.8.8.8/x",
            // CGNAT 100.64.0.0/10 边界外
            "http://100.63.255.255/x",
            "http://100.128.0.0/x",
            // 多播/保留段之前最后一个公网单播
            "http://223.255.255.255/x",
            // 公网 IPv6 单播
            "http://[2606:4700:4700::1111]/x",
            // IPv4-mapped 公网
            "http://[::ffff:8.8.8.8]/x",
        ] {
            assert!(
                validate_url_sync(url).is_ok(),
                "must allow public literal: {}",
                url
            );
        }
        for url in ["http://192.0.0.9/x", "http://192.0.0.10/x"] {
            assert!(
                validate_url_sync(url).is_ok(),
                "must allow IANA globally-reachable protocol address: {}",
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

    /// SSRF：::/96 内已废弃的 IPv4-compatible 形式（::a.b.c.d，非 :: 与 ::1
    /// 的 <32 位前缀形式）必须拒绝，防止绕过 IPv4 检查。
    #[test]
    fn validate_url_rejects_ipv4_compatible_ipv6() {
        for url in [
            "http://[::10.0.0.5]/x",
            "http://[::127.0.0.1]/x",
            // 同段的等价 hex 书写形式（url crate 规范化后同形）
            "http://[::0a00:5]/x",
            "http://[::7f00:1]/x",
        ] {
            assert!(
                validate_url_sync(url).is_err(),
                "must reject IPv4-compatible IPv6: {}",
                url
            );
        }
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

    /// 混合 DNS rebinding（可测函数级验证）：解析结果中只要混入任一非全球
    /// 可路由地址即整体拒绝；纯公网结果放行。
    #[test]
    fn ensure_all_globally_routable_rejects_mixed_dns_answers() {
        let host = "mixed.example.test";
        let public_1 = SocketAddr::new(Ipv4Addr::new(8, 8, 8, 8).into(), 80);
        let public_2 = SocketAddr::new(Ipv4Addr::new(1, 1, 1, 1).into(), 80);
        // 基准测试段（198.18.0.0/15）：旧黑名单放行、白名单拒绝
        let benchmark = SocketAddr::new(Ipv4Addr::new(198, 18, 0, 1).into(), 80);

        assert!(
            ensure_all_globally_routable(host, &[public_1, public_2]).is_ok(),
            "pure public answers must pass"
        );
        let err = ensure_all_globally_routable(host, &[public_1, benchmark])
            .expect_err("mixed list with a non-globally-routable address must be rejected");
        assert!(
            format!("{}", err).contains("198.18.0.1"),
            "error must name the offending address: {}",
            err
        );
    }

    /// DNS 失败 fail-closed：不存在的域名必须拒绝（不能保守放行）。
    /// 依赖 `.invalid` TLD（RFC 2606 保留、不会分配）本地解析失败；若 CI
    /// 环境存在 DNS 劫持/通配解析导致解析"成功"，请给本测试加 `#[ignore]`。
    #[tokio::test]
    async fn validate_url_rejects_unresolvable_host() {
        let result = validate_url("http://nonexistent-host-clashedge-test.invalid/").await;
        assert!(
            result.is_err(),
            "unresolvable host must be rejected (fail-closed), got {:?}",
            result
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
    // v1.0.5 修复：改用 SOCKS5 本地解析 + resolve pinning。代理收到已验证 IP，
    // 原 URL hostname 则保留给 HTTP Host / TLS SNI 与证书 hostname 校验。

    #[tokio::test]
    async fn direct_non_success_retries_through_pinned_socks5_fallback() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let direct_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let direct_addr = direct_listener.local_addr().unwrap();
        let direct_task = tokio::spawn(async move {
            let (mut conn, _) = direct_listener.accept().await.unwrap();
            let mut request = [0u8; 1024];
            let _ = conn.read(&mut request).await.unwrap();
            conn.write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n")
                .await
                .unwrap();
        });

        let socks_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let socks_port = socks_listener.local_addr().unwrap().port();
        let socks_task = tokio::spawn(async move {
            let (mut conn, _) = socks_listener.accept().await.unwrap();
            let mut greeting = [0u8; 2];
            conn.read_exact(&mut greeting).await.unwrap();
            let mut methods = vec![0u8; greeting[1] as usize];
            conn.read_exact(&mut methods).await.unwrap();
            conn.write_all(&[5, 0]).await.unwrap();
            let mut header = [0u8; 4];
            conn.read_exact(&mut header).await.unwrap();
            assert_eq!(header[3], 1, "fallback must send a pinned IPv4 target");
            let mut target = [0u8; 6];
            conn.read_exact(&mut target).await.unwrap();
            assert_eq!(&target[..4], &[203, 0, 113, 9]);
            conn.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0])
                .await
                .unwrap();
            let mut request = [0u8; 1024];
            let size = conn.read(&mut request).await.unwrap();
            assert!(String::from_utf8_lossy(&request[..size]).contains("fallback.example.test"));
            conn.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nOK")
                .await
                .unwrap();
        });

        let url = "http://fallback.example.test/resource";
        let direct = apply_route(
            build_client_with_resolved(url, &[direct_addr], Some(Duration::from_secs(5))).unwrap(),
            FetchRoute::Direct,
            "",
        )
        .unwrap()
        .redirect(no_redirect_policy())
        .build()
        .unwrap();
        let proxy_url = format!("socks5://127.0.0.1:{}", socks_port);
        let pinned = SocketAddr::new(Ipv4Addr::new(203, 0, 113, 9).into(), 80);
        let proxied = apply_route(
            build_client_with_resolved(url, &[pinned], Some(Duration::from_secs(5))).unwrap(),
            FetchRoute::LocalProxy,
            &proxy_url,
        )
        .unwrap()
        .redirect(no_redirect_policy())
        .build()
        .unwrap();

        let response = send_direct_then_proxy(
            &direct,
            &proxied,
            url,
            &proxy_url,
            Some(Duration::from_secs(5)),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        direct_task.await.unwrap();
        socks_task.await.unwrap();
    }

    /// integration：SOCKS5 fallback 必须同时满足：
    /// 1) SOCKS connect 目标是 validate_url 已验证的 IP（代理端不做 DNS）；
    /// 2) TLS ClientHello SNI 仍是原 hostname（证书 hostname 校验不被破坏）。
    #[tokio::test]
    async fn socks5_fallback_pins_ip_but_preserves_tls_sni() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_port = listener.local_addr().unwrap().port();
        let proxy_task = tokio::spawn(async move {
            let (mut conn, _) = listener.accept().await.unwrap();
            let mut greeting = [0u8; 2];
            conn.read_exact(&mut greeting).await.unwrap();
            assert_eq!(greeting[0], 5);
            let mut methods = vec![0u8; greeting[1] as usize];
            conn.read_exact(&mut methods).await.unwrap();
            conn.write_all(&[5, 0]).await.unwrap();

            let mut request = [0u8; 4];
            conn.read_exact(&mut request).await.unwrap();
            assert_eq!(&request[..3], &[5, 1, 0]);
            assert_eq!(
                request[3], 1,
                "SOCKS target must be IPv4, not a proxy-resolved domain"
            );
            let mut ip_and_port = [0u8; 6];
            conn.read_exact(&mut ip_and_port).await.unwrap();
            conn.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0])
                .await
                .unwrap();

            let mut client_hello = Vec::new();
            let mut chunk = [0u8; 2048];
            for _ in 0..8 {
                match tokio::time::timeout(Duration::from_millis(250), conn.read(&mut chunk)).await
                {
                    Ok(Ok(0)) | Err(_) => break,
                    Ok(Ok(size)) => {
                        client_hello.extend_from_slice(&chunk[..size]);
                        if client_hello
                            .windows(b"tls-name.example.test".len())
                            .any(|window| window == b"tls-name.example.test")
                        {
                            break;
                        }
                    }
                    Ok(Err(e)) => panic!("failed reading TLS ClientHello: {}", e),
                }
            }
            (ip_and_port, client_hello)
        });

        let url = "https://tls-name.example.test/path";
        let resolved = vec![SocketAddr::new(Ipv4Addr::new(203, 0, 113, 7).into(), 443)];
        let builder =
            build_client_with_resolved(url, &resolved, Some(Duration::from_secs(5))).unwrap();
        let proxy_url = format!("socks5://127.0.0.1:{}", proxy_port);
        let client = apply_route(builder, FetchRoute::LocalProxy, &proxy_url)
            .unwrap()
            .redirect(no_redirect_policy())
            .build()
            .unwrap();

        let send_result = client
            .get(url)
            .header(USER_AGENT, user_agent())
            .send()
            .await;

        let (ip_and_port, client_hello) = tokio::time::timeout(Duration::from_secs(3), proxy_task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&ip_and_port[..4], &[203, 0, 113, 7]);
        assert_eq!(u16::from_be_bytes([ip_and_port[4], ip_and_port[5]]), 443);
        assert!(
            client_hello
                .windows(b"tls-name.example.test".len())
                .any(|window| window == b"tls-name.example.test"),
            "TLS ClientHello must preserve the original hostname as SNI (send={:?}, {} bytes: {:02x?})",
            send_result,
            client_hello.len(),
            &client_hello[..client_hello.len().min(96)]
        );
    }

    /// 重定向链共享总 deadline：任何一跳（含首跳）都不得越过整链总预算。
    ///
    /// 测试设计说明：第二跳重定向目标若用伪造主机名，`validate_url` 的真实
    /// DNS 查询会先行失败（`.test` TLD 无解析），连接根本到不了本地 listener；
    /// 因此用「首跳慢速」验证共享 deadline 的 per-hop 代码路径（与重定向跳
    /// 同一条逻辑：剩余时间检查 → client 超时 → tokio timeout 包裹），并用
    /// remaining 阈值分支与 None（streaming）模式补齐行为断言。
    #[tokio::test]
    async fn redirect_chain_enforces_shared_total_deadline() {
        use tokio::io::AsyncReadExt;

        // 慢速服务器：accept 后读取请求头，然后静默不响应（挂死的一跳）
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            while let Ok((mut conn, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut buf = [0u8; 1024];
                    let _ = conn.read(&mut buf).await;
                    // 保持静默，永不响应
                    tokio::time::sleep(Duration::from_secs(10)).await;
                });
            }
        });

        let url = "http://slow-redirect.example.test/first";
        // client 不设总超时（等价 streaming 模式）：只能靠共享 deadline 兜底
        let client = apply_route(
            build_client_with_resolved(url, &[addr], None).unwrap(),
            FetchRoute::Direct,
            "",
        )
        .unwrap()
        .redirect(no_redirect_policy())
        .build()
        .unwrap();

        // 场景 A：总 deadline = 1s —— 必须在 ~1s（容忍 +500ms）内返回
        // 含超时语义的错误，而不是挂死。
        let start = Instant::now();
        let result = send_and_follow(
            &client,
            url,
            FetchRoute::Direct,
            "",
            Some(Duration::from_secs(1)),
        )
        .await;
        let elapsed = start.elapsed();
        let err = result.expect_err("total deadline must abort the request");
        assert!(
            format!("{}", err).contains("timed out"),
            "error must carry timeout semantics: {}",
            err
        );
        assert!(
            elapsed >= Duration::from_millis(900),
            "must not fail before the deadline: {:?}",
            elapsed
        );
        assert!(
            elapsed <= Duration::from_millis(1500),
            "must respect the 1s deadline (+500ms tolerance): {:?}",
            elapsed
        );

        // 场景 B：None（streaming 模式）不受 deadline 约束——服务器静默、
        // 无 client 总超时、read_timeout 只约束单次 read 间隔，因此请求在
        // 1.5s 内保持挂起（不会"提前失败"也不会"成功返回"）。
        let pending = tokio::time::timeout(
            Duration::from_millis(1500),
            send_and_follow(&client, url, FetchRoute::Direct, "", None),
        )
        .await;
        assert!(
            pending.is_err(),
            "None deadline mode must stay pending beyond 1.5s, got {:?}",
            pending
        );

        // 场景 C：剩余时间不足阈值（100ms）——发送前直接判总超时，立即返回。
        let start = Instant::now();
        let result = send_and_follow(
            &client,
            url,
            FetchRoute::Direct,
            "",
            Some(Duration::from_millis(50)),
        )
        .await;
        let err = result.expect_err("remaining below the minimum threshold must abort");
        assert!(
            format!("{}", err).contains("timed out"),
            "threshold error must carry timeout semantics: {}",
            err
        );
        assert!(
            start.elapsed() <= Duration::from_millis(200),
            "threshold branch must be immediate, took {:?}",
            start.elapsed()
        );

        server.abort();
    }

    /// 手工 Release Gate：通过真实 Mihomo mixed-port 跑生产 fallback 路径，覆盖
    /// GitHub Release redirect、常见订阅 CDN 与 geodata CDN。默认忽略；执行时
    /// 设置 CLASHEDGE_TEST_PROXY=socks5://127.0.0.1:<isolated-port>。
    #[tokio::test]
    #[ignore = "requires an isolated real Mihomo and internet access"]
    async fn real_https_fallback_targets_keep_tls_and_redirect_safety() {
        let proxy_url = std::env::var("CLASHEDGE_TEST_PROXY")
            .expect("set CLASHEDGE_TEST_PROXY to an isolated Mihomo SOCKS5 mixed-port");
        for url in [
            "https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/geoip.dat",
            "https://raw.githubusercontent.com/akaspyrean/external/main/rules/direct.yaml",
            "https://cdn.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@release/geosite.dat",
        ] {
            let resolved = validate_url(url).await.unwrap();
            let client = apply_route(
                build_client_with_resolved(url, &resolved, Some(Duration::from_secs(45))).unwrap(),
                FetchRoute::LocalProxy,
                &proxy_url,
            )
            .unwrap()
            .redirect(no_redirect_policy())
            .build()
            .unwrap();
            let response = send_and_follow(
                &client,
                url,
                FetchRoute::LocalProxy,
                &proxy_url,
                Some(Duration::from_secs(45)),
            )
            .await
            .unwrap_or_else(|e| panic!("real proxy fetch failed for {}: {}", url, e));
            assert!(response.status().is_success(), "non-success for {}", url);
        }
    }
}
