// src-tauri/src/util/fetch/guards.rs
//! URL 校验与 SSRF 白名单规则：`validate_url`（异步，含 DNS 检查）、
//! 同步字面量预检、全球可路由地址白名单判定、URL 脱敏。

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use reqwest::Url;

use crate::util::error::{Error, Result};

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
}
