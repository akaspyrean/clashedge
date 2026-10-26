// src-tauri/src/util/geoip.rs
//! IP → 国家代码（读取 `Data/Country.mmdb`）。
//!
//! 只用于「节点名无法推断地区」时的兜底：匿名节点名（例如
//! `JMS-1416430@c37s1...`）不含国家信息，但其 `server` 字段指向真实机房，
//! 解析成 IP 后查本地 MaxMind DB 即可得到国家。
//!
//! 设计约束：
//! - **纯本地**：不调用任何在线地理 API；数据库随 GeoData 一起分发。
//! - **容错**：数据库缺失 / 损坏、DNS 失败都返回 `None`，绝不阻塞代理功能。
//! - **缓存**：DB reader 按路径 + mtime 缓存；主机 → 国家结果带 TTL，
//!   避免每次打开代理页都重新 DNS + 查库。

use std::collections::HashMap;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use maxminddb::{path, Reader};
use once_cell::sync::Lazy;
use parking_lot::Mutex;

/// 单次系统 DNS 解析上限：节点格加载不能因为某个坏域名卡住。
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(1);
/// 成功结果的缓存时间（机房 IP 短期内不会频繁跨国迁移）。
const HIT_TTL: Duration = Duration::from_secs(6 * 60 * 60);
/// 失败结果的短缓存：避免页面轮询反复打 DNS，同时允许网络恢复后重试。
const MISS_TTL: Duration = Duration::from_secs(5 * 60);
/// 并发解析上限，避免节点很多时瞬间发起大量 DNS 查询。
pub const MAX_CONCURRENT_LOOKUPS: usize = 8;

struct DbCache {
    path: PathBuf,
    modified: SystemTime,
    reader: Reader<Vec<u8>>,
}

type CountryCache = HashMap<String, (Option<String>, Instant)>;

static DB: Lazy<Mutex<Option<DbCache>>> = Lazy::new(|| Mutex::new(None));
static COUNTRY_CACHE: Lazy<Mutex<CountryCache>> = Lazy::new(|| Mutex::new(HashMap::new()));

/// 直接用 IP 查国家代码（ISO 3166-1 alpha-2，例如 `US` / `HK`）。
pub fn country_code_for_ip(db: &Path, ip: IpAddr) -> Option<String> {
    let mut guard = DB.lock();
    let modified = std::fs::metadata(db).ok()?.modified().ok()?;
    let reload = guard
        .as_ref()
        .map(|c| c.path != db || c.modified != modified)
        .unwrap_or(true);
    if reload {
        let reader = Reader::open_readfile(db).ok()?;
        *guard = Some(DbCache {
            path: db.to_path_buf(),
            modified,
            reader,
        });
    }

    let reader = &guard.as_ref()?.reader;
    let result = reader.lookup(ip).ok()?;
    // 优先使用 MaxMind 的 country 字段；某些记录（云/CDN 段）只有
    // registered_country，或把组织名塞进 iso_code（如 GOOGLE/CLOUDFLARE）。
    // 这里只接受真正像 ISO 3166-1 alpha-2 的代码，避免把组织名当国家。
    let primary: Option<String> = result
        .decode_path(&path!["country", "iso_code"])
        .ok()
        .flatten();
    let registered: Option<String> = result
        .decode_path(&path!["registered_country", "iso_code"])
        .ok()
        .flatten();
    primary
        .into_iter()
        .chain(registered)
        .find(|code| is_country_code(code))
}

/// 是否为可展示的两位国家代码（排除 MaxMind 的 A1/A2/O1 与 EU/AP 等非国家代码）。
fn is_country_code(code: &str) -> bool {
    code.len() == 2
        && code.chars().all(|c| c.is_ascii_alphabetic())
        && !matches!(code.to_ascii_uppercase().as_str(), "EU" | "AP")
}

/// 节点 `server`（域名或 IP 字面量）→ 国家代码。
///
/// 域名先做一次解析，再查本地库；结果按主机名缓存。
///
/// `fake_ip_range` 来自当前配置（TUN + fake-ip 时非空）：系统解析若落入该段，
/// 说明拿到的是 mihomo 的假地址，必须改用 DoH 取真实 IP 再查库。
pub async fn country_code_for_server(
    db: &Path,
    server: &str,
    fake_ip_range: Option<&ipnet::IpNet>,
) -> Option<String> {
    let host = normalize_host(server)?;

    if let Ok(ip) = host.parse::<IpAddr>() {
        return country_code_for_ip(db, ip);
    }

    if let Some(cached) = cached_country(&host) {
        return cached;
    }

    // 1) 系统解析（快）：只接受不在 fake-ip 段内的地址。
    let mut chosen = None;
    if let Ok(Ok(addrs)) =
        tokio::time::timeout(RESOLVE_TIMEOUT, tokio::net::lookup_host((host.as_str(), 0))).await
    {
        let real = addrs
            .map(|addr| addr.ip())
            .filter(|ip| fake_ip_range.map(|net| !net.contains(ip)).unwrap_or(true));
        chosen = prefer_ipv4(real);
    }

    // 2) TUN + fake-ip（或被污染/解析失败）：DoH 取真实 A/AAAA，经 IP 端点出站。
    if chosen.is_none() {
        if let Ok(addrs) = crate::util::fetch::resolve_real_addrs(&host).await {
            chosen = prefer_ipv4(addrs.into_iter());
        }
    }

    let code = country_code_for_ip(db, chosen?);
    cache_country(host, code.clone());
    code
}

/// 去掉端口、IPv6 方括号和空白；空主机名返回 `None`。
fn normalize_host(server: &str) -> Option<String> {
    let trimmed = server.trim();
    if trimmed.is_empty() {
        return None;
    }
    // 裸 IP 字面量（IPv4 / IPv6，无端口）
    if trimmed.parse::<IpAddr>().is_ok() {
        return Some(trimmed.to_string());
    }
    // `[2001:db8::1]:443` / `[2001:db8::1]`
    if let Some(rest) = trimmed.strip_prefix('[') {
        let end = rest.find(']')?;
        let host = &rest[..end];
        return (!host.is_empty()).then(|| host.to_string());
    }
    // `host:port`（域名不会包含冒号；裸 IPv6 已在上面的 parse 分支处理）
    let host = trimmed.split(':').next().unwrap_or(trimmed);
    (!host.is_empty()).then(|| host.to_string())
}

/// 优先取 IPv4（GeoIP Country 库对 IPv4 覆盖更稳定），否则退回任意地址。
fn prefer_ipv4(addrs: impl Iterator<Item = IpAddr>) -> Option<IpAddr> {
    let mut fallback = None;
    for ip in addrs {
        if ip.is_ipv4() {
            return Some(ip);
        }
        fallback.get_or_insert(ip);
    }
    fallback
}

fn cached_country(host: &str) -> Option<Option<String>> {
    let guard = COUNTRY_CACHE.lock();
    let (code, at) = guard.get(host)?;
    let ttl = if code.is_some() { HIT_TTL } else { MISS_TTL };
    if at.elapsed() < ttl {
        Some(code.clone())
    } else {
        None
    }
}

fn cache_country(host: String, code: Option<String>) {
    let mut guard = COUNTRY_CACHE.lock();
    // 上限保护：极端订阅（上千节点）也不让缓存无限增长。
    if guard.len() >= 4096 {
        guard.clear();
    }
    guard.insert(host, (code, Instant::now()));
}

#[cfg(test)]
mod tests {
    use super::{country_code_for_ip, country_code_for_server, normalize_host, prefer_ipv4};
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
    use std::path::Path;

    #[test]
    fn normalizes_hosts_and_strips_ports() {
        assert_eq!(normalize_host(" example.com "), Some("example.com".into()));
        assert_eq!(
            normalize_host("example.com:443"),
            Some("example.com".into())
        );
        assert_eq!(normalize_host("1.2.3.4:8388"), Some("1.2.3.4".into()));
        assert_eq!(normalize_host("[2001:db8::1]"), Some("2001:db8::1".into()));
        assert_eq!(
            normalize_host("[2001:db8::1]:443"),
            Some("2001:db8::1".into())
        );
        assert_eq!(normalize_host("2001:db8::1"), Some("2001:db8::1".into()));
        assert_eq!(normalize_host("   "), None);
    }

    #[test]
    fn prefers_ipv4_but_falls_back_to_ipv6() {
        let v4: IpAddr = Ipv4Addr::new(203, 0, 113, 1).into();
        let v6: IpAddr = Ipv6Addr::LOCALHOST.into();
        assert_eq!(prefer_ipv4(vec![v6, v4].into_iter()), Some(v4));
        assert_eq!(prefer_ipv4(vec![v6].into_iter()), Some(v6));
        assert_eq!(prefer_ipv4(std::iter::empty()), None);
    }

    #[test]
    fn only_accepts_two_letter_country_codes() {
        assert!(super::is_country_code("US"));
        assert!(super::is_country_code("hk"));
        assert!(!super::is_country_code("GOOGLE"));
        assert!(!super::is_country_code("CLOUDFLARE"));
        assert!(!super::is_country_code("A1"));
        assert!(!super::is_country_code("EU"));
        assert!(!super::is_country_code(""));
    }

    /// 真实库冒烟测试（CI 没有 Country.mmdb 时跳过）：设置 COUNTRY_MMDB 后运行。
    #[test]
    fn real_database_lookup_when_available() {
        let Ok(path) = std::env::var("COUNTRY_MMDB") else {
            return;
        };
        let ip: IpAddr = "5.9.0.1".parse().unwrap();
        assert_eq!(
            country_code_for_ip(Path::new(&path), ip).as_deref(),
            Some("DE")
        );
    }

    #[tokio::test]
    async fn server_ip_literal_uses_local_database_when_available() {
        let Ok(path) = std::env::var("COUNTRY_MMDB") else {
            return;
        };
        assert_eq!(
            country_code_for_server(Path::new(&path), "5.9.0.1:443", None)
                .await
                .as_deref(),
            Some("DE")
        );
    }

    /// 强制把系统解析结果视为 fake-ip，验证 DoH 兜底路径（需网络 + COUNTRY_MMDB）。
    #[tokio::test]
    async fn domain_uses_doh_when_system_dns_is_fake() {
        let Ok(path) = std::env::var("COUNTRY_MMDB") else {
            return;
        };
        let all: ipnet::IpNet = "0.0.0.0/0".parse().unwrap();
        assert_eq!(
            country_code_for_server(Path::new(&path), "www.hetzner.com", Some(&all))
                .await
                .as_deref(),
            Some("DE")
        );
    }
}
