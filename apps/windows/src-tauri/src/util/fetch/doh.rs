// src-tauri/src/util/fetch/doh.rs
//! TUN + fake-ip 场景下的"真实地址"校验（SSRF 守卫的补强）。
//!
//! 开启 TUN 且 `dns.enhanced-mode: fake-ip` 时，本机系统解析会被 mihomo 劫持，任何
//! 主机名都解析到 fake-ip 段。此时本地解析结果无法用于判断目标是否为内网，
//! 如果直接放行，`nas.lan`、`router.asus.com` 这类解析到内网的名字就能经由订阅里的
//! 重定向 / provider URL 被 mihomo 直连访问（盲 SSRF）。
//!
//! 这里改用 DoH 取**真实**记录：请求目标是 IP 字面量（无需再做 DNS），经 TUN 正常出站。
//! 取不到答案一律 fail-closed——"无法确认目标是公网"不能当作"安全"。
//! 结果只用于校验（mihomo 连接时仍会自行解析，TOCTOU 窗口比"完全不校验"小得多）。
//!
//! 优化点：
//! - 端点内 A/AAAA 并行查询（tokio::join!），单端点超时 2s。
//! - 进程内 TTL 缓存（60s，只缓存成功结果），用于 SSRF 校验路径（verify_real_addrs 的输入）。
//! - 缓存值不得用于连接钉定（fake-ip 路径本来就返回空列表走 socks5h，不受影响）。

use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::util::error::{Error, Result};
use once_cell::sync::Lazy;

/// DoH JSON 端点（IP 字面量，证书含 IP SAN）。依次尝试（默认值，可被运行时覆盖）。
const DEFAULT_DOH_ENDPOINTS: &[&str] = &["https://223.5.5.5/resolve", "https://1.1.1.1/dns-query"];
/// 单次 DoH 查询超时（端点内 A/AAAA 并行，两者取最长）。从 4s 收紧到 2s。
const DOH_TIMEOUT: Duration = Duration::from_secs(2);
/// 缓存 TTL：60s。只缓存成功结果，失败不缓存（保证网络恢复后立即可重试）。
const CACHE_TTL: Duration = Duration::from_secs(60);

/// 解析 DoH JSON（RFC 8484 JSON 变体：`Answer[].{type,data}`）里的 A / AAAA 记录。
pub(super) fn parse_answers(body: &serde_json::Value) -> Vec<IpAddr> {
    body.get("Answer")
        .and_then(|a| a.as_array())
        .map(|arr| {
            arr.iter()
                .filter(|r| matches!(r.get("type").and_then(|t| t.as_u64()), Some(1) | Some(28)))
                .filter_map(|r| r.get("data").and_then(|d| d.as_str()))
                .filter_map(|d| d.parse::<IpAddr>().ok())
                .collect()
        })
        .unwrap_or_default()
}

/// 缓存条目：(缓存写入时间, 地址列表)。
type CacheEntry = (Instant, Vec<IpAddr>);

/// 进程内 TTL 缓存：host -> CacheEntry。
/// 只用于 SSRF 校验（verify_real_addrs 的输入），不得用于连接钉定。
static DOH_CACHE: Lazy<Mutex<std::collections::HashMap<String, CacheEntry>>> =
    Lazy::new(|| Mutex::new(std::collections::HashMap::new()));

/// 可配置的 DoH 端点列表（默认两个内置 IP 端点，支持运行时覆盖）。
static DOH_ENDPOINTS: Lazy<Mutex<Vec<String>>> = Lazy::new(|| {
    Mutex::new(
        DEFAULT_DOH_ENDPOINTS
            .iter()
            .map(|s| s.to_string())
            .collect(),
    )
});

/// 更新 DoH 端点列表（供配置热重载调用）。空列表 → 恢复默认。
pub fn set_doh_endpoints(endpoints: Vec<String>) {
    let mut guard = DOH_ENDPOINTS.lock().unwrap();
    if endpoints.is_empty() {
        *guard = vec![
            "https://223.5.5.5/resolve".to_string(),
            "https://1.1.1.1/dns-query".to_string(),
        ];
    } else {
        *guard = endpoints;
    }
}

async fn query(
    client: &reqwest::Client,
    endpoint: &str,
    host: &str,
    rtype: &str,
) -> Result<Vec<IpAddr>> {
    let resp = client
        .get(endpoint)
        .query(&[("name", host), ("type", rtype)])
        .header("accept", "application/dns-json")
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(Error::Other(format!("DoH HTTP {}", resp.status())));
    }
    let body: serde_json::Value = resp.json().await?;
    Ok(parse_answers(&body))
}

/// 端点内 A/AAAA 并行查询：两个请求并发发起，任一成功且非空即接受（A 优先）。
async fn query_endpoint(
    client: &reqwest::Client,
    endpoint: &str,
    host: &str,
) -> Result<Vec<IpAddr>> {
    let (a_res, aaaa_res) = tokio::join!(
        query(client, endpoint, host, "A"),
        query(client, endpoint, host, "AAAA")
    );
    match (a_res, aaaa_res) {
        (Ok(mut v4), Ok(v6)) => {
            v4.extend(v6);
            Ok(v4)
        }
        // A 记录成功且非空即可；AAAA 失败（部分端点不支持）不否决结果
        (Ok(v4), Err(_)) if !v4.is_empty() => Ok(v4),
        // A 失败但 AAAA 成功：按 AAAA 返回（极少见，保底）
        (Err(_), Ok(v6)) if !v6.is_empty() => Ok(v6),
        (Err(e), _) | (_, Err(e)) => Err(e),
    }
}

/// 取 `host` 的真实 A + AAAA 地址。全部端点失败 → Err（fail-closed）。
/// 命中缓存（≤60s）直接返回，不再发起网络请求。
pub async fn resolve_real_addrs(host: &str) -> Result<Vec<IpAddr>> {
    let endpoints = {
        let guard = DOH_ENDPOINTS.lock().unwrap();
        guard.clone()
    };
    resolve_with_endpoints(host, &endpoints).await
}

/// 核心实现（可测）：按顺序回退尝试 `endpoints`，端点内 A/AAAA 并行。
/// 端点间串行：避免双端点同时打不可达网络。
async fn resolve_with_endpoints(host: &str, endpoints: &[String]) -> Result<Vec<IpAddr>> {
    // 缓存查找
    {
        let mut cache = DOH_CACHE.lock().unwrap();
        if let Some((cached_at, addrs)) = cache.get(host) {
            if cached_at.elapsed() < CACHE_TTL {
                return Ok(addrs.clone());
            }
            // 过期移除
            cache.remove(host);
        }
    }

    let client = reqwest::Client::builder()
        .timeout(DOH_TIMEOUT)
        .no_proxy()
        .build()?;

    let mut last: Option<Error> = None;
    for ep in endpoints {
        let res = query_endpoint(&client, ep, host).await;
        match res {
            Ok(addrs) if !addrs.is_empty() => {
                // 写入缓存（只缓存成功结果）
                let mut cache = DOH_CACHE.lock().unwrap();
                cache.insert(host.to_string(), (Instant::now(), addrs.clone()));
                return Ok(addrs);
            }
            Ok(_) => {
                // 空结果不缓存，继续尝试下一端点
                last = Some(Error::Other("DoH empty response".into()));
            }
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| Error::Other("no DoH endpoint available".into())))
}

/// 仅供测试：清空缓存。
#[cfg(test)]
fn clear_cache() {
    DOH_CACHE.lock().unwrap().clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn parses_a_and_aaaa_and_ignores_cname() {
        let body = serde_json::json!({
            "Status": 0,
            "Answer": [
                {"name": "x.", "type": 5, "data": "y.example."},
                {"name": "y.", "type": 1, "data": "93.184.216.34"},
                {"name": "y.", "type": 28, "data": "2606:2800:220:1::1"},
                {"name": "y.", "type": 1, "data": "not-an-ip"}
            ]
        });
        let got = parse_answers(&body);
        assert_eq!(got.len(), 2);
        assert!(got.contains(&"93.184.216.34".parse().unwrap()));
    }

    #[test]
    fn missing_answer_is_empty() {
        assert!(parse_answers(&serde_json::json!({"Status": 3})).is_empty());
    }

    /// 端点内 A/AAAA 并行：两个查询并发发起，总耗时≈max(单查耗时) 而非 sum。
    /// 用本地 mock TCP listener 模拟延迟：A 延迟 100ms，AAAA 延迟 150ms → 并行应在 ~200ms 内完成。
    #[tokio::test]
    async fn endpoint_parallel_a_aaaa() {
        clear_cache();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            loop {
                let (mut sock, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 1024];
                let _n = tokio::io::AsyncReadExt::read(&mut sock, &mut buf)
                    .await
                    .unwrap();
                let req = String::from_utf8_lossy(&buf);
                // 注意 "type=A" 是 "type=AAAA" 的子串，必须先判 AAAA
                let is_aaaa = req.contains("type=AAAA");
                let is_a = !is_aaaa && req.contains("type=A");
                // A 延迟 100ms，AAAA 延迟 150ms
                let delay = if is_a {
                    100
                } else if is_aaaa {
                    150
                } else {
                    0
                };
                tokio::time::sleep(Duration::from_millis(delay)).await;
                let body = if is_a {
                    r#"{"Status":0,"Answer":[{"name":"example.com.","type":1,"data":"93.184.216.34"}]}"#
                } else if is_aaaa {
                    r#"{"Status":0,"Answer":[{"name":"example.com.","type":28,"data":"2606:2800:220:1::1"}]}"#
                } else {
                    "{}"
                };
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/dns-json\r\nContent-Length: {}\r\n\r\n{}",
                    body.len(),
                    body
                );
                tokio::io::AsyncWriteExt::write_all(&mut sock, resp.as_bytes())
                    .await
                    .unwrap();
                tokio::io::AsyncWriteExt::shutdown(&mut sock).await.unwrap();
            }
        });

        let mock_endpoint = format!("http://{addr}/dns-query");
        let start = Instant::now();
        let res = resolve_with_endpoints("example.com", &[mock_endpoint]).await;
        let elapsed = start.elapsed();

        server.abort();
        assert!(res.is_ok(), "query failed: {:?}", res.err());
        let addrs = res.unwrap();
        assert_eq!(addrs.len(), 2);
        // 并行：应≈max(100,150)=150ms，给 100ms 余量
        assert!(
            elapsed < Duration::from_millis(300),
            "expected parallel (~150ms), got {:?}",
            elapsed
        );
    }

    /// 缓存命中：第二次调用不发起网络请求（用 mock listener 计数验证）。
    #[tokio::test]
    async fn cache_hit_skips_network() {
        clear_cache();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let hit_count = std::sync::Arc::new(std::sync::Mutex::new(0usize));
        let hit_count_clone = hit_count.clone();

        let server = tokio::spawn(async move {
            // 最多接 4 个连接（A + AAAA 首次 2 个；若缓存失效第二次调用再来 2 个）
            for _ in 0..4 {
                let (mut sock, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 1024];
                let _n = tokio::io::AsyncReadExt::read(&mut sock, &mut buf)
                    .await
                    .unwrap();
                *hit_count_clone.lock().unwrap() += 1;
                let body =
                    r#"{"Status":0,"Answer":[{"name":"cache.test.","type":1,"data":"1.2.3.4"}]}"#;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/dns-json\r\nContent-Length: {}\r\n\r\n{}",
                    body.len(),
                    body
                );
                tokio::io::AsyncWriteExt::write_all(&mut sock, resp.as_bytes())
                    .await
                    .unwrap();
                tokio::io::AsyncWriteExt::shutdown(&mut sock).await.unwrap();
            }
        });

        let mock_endpoint = format!("http://{addr}/dns-query");
        // 第一次查询（A + AAAA 并行 = 2 个连接）
        let _ = resolve_with_endpoints("cache.test", std::slice::from_ref(&mock_endpoint)).await;
        // 立即第二次查询（应命中缓存，不再请求网络）
        let _ = resolve_with_endpoints("cache.test", std::slice::from_ref(&mock_endpoint)).await;

        server.abort();
        let hits = *hit_count.lock().unwrap();
        assert_eq!(
            hits, 2,
            "expected 2 network hits (A+AAAA on 1st call, cache hit on 2nd), got {hits}"
        );
    }

    /// 失败不缓存：首次失败后网络恢复，第二次应再次尝试网络（而非返回缓存的 Err）。
    #[tokio::test]
    async fn failure_not_cached() {
        clear_cache();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let hit_count = std::sync::Arc::new(std::sync::Mutex::new(0usize));
        let hit_count_clone = hit_count.clone();

        // 第一阶段：服务器返回 500
        let server1 = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 1024];
            let _n = tokio::io::AsyncReadExt::read(&mut sock, &mut buf)
                .await
                .unwrap();
            *hit_count_clone.lock().unwrap() += 1;
            let resp = "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n";
            tokio::io::AsyncWriteExt::write_all(&mut sock, resp.as_bytes())
                .await
                .unwrap();
            tokio::io::AsyncWriteExt::shutdown(&mut sock).await.unwrap();
        });

        let mock_endpoint = format!("http://{addr}/dns-query");
        let _ = resolve_with_endpoints("fail.test", &[mock_endpoint]).await;
        server1.await.unwrap();

        // 第二阶段：服务器恢复正常
        let listener2 = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr2 = listener2.local_addr().unwrap();
        let hit_count2 = std::sync::Arc::new(std::sync::Mutex::new(0usize));
        let hit_count2_clone = hit_count2.clone();
        let server2 = tokio::spawn(async move {
            let (mut sock, _) = listener2.accept().await.unwrap();
            let mut buf = [0u8; 1024];
            let _n = tokio::io::AsyncReadExt::read(&mut sock, &mut buf)
                .await
                .unwrap();
            *hit_count2_clone.lock().unwrap() += 1;
            let body =
                r#"{"Status":0,"Answer":[{"name":"recover.test.","type":1,"data":"5.6.7.8"}]}"#;
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/dns-json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            tokio::io::AsyncWriteExt::write_all(&mut sock, resp.as_bytes())
                .await
                .unwrap();
            tokio::io::AsyncWriteExt::shutdown(&mut sock).await.unwrap();
        });

        let mock_endpoint2 = format!("http://{addr2}/dns-query");
        let res = resolve_with_endpoints("recover.test", &[mock_endpoint2]).await;
        server2.await.unwrap();

        assert!(res.is_ok());
        let hits1 = *hit_count.lock().unwrap();
        let hits2 = *hit_count2.lock().unwrap();
        assert_eq!(hits1, 1, "first attempt hit network");
        assert_eq!(
            hits2, 1,
            "second attempt (after failure) should hit network again, not cache"
        );
    }

    /// A-only 成功路径：A 成功非空、AAAA 失败 → 返回 A 结果（不否决）。
    #[tokio::test]
    async fn a_only_success_path() {
        clear_cache();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut sock, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 1024];
                let _n = tokio::io::AsyncReadExt::read(&mut sock, &mut buf)
                    .await
                    .unwrap();
                let req = String::from_utf8_lossy(&buf);
                // 注意 "type=A" 是 "type=AAAA" 的子串，必须先判 AAAA
                let is_aaaa = req.contains("type=AAAA");
                let is_a = !is_aaaa && req.contains("type=A");
                let (status, body) = if is_a {
                    (
                        "200 OK",
                        r#"{"Status":0,"Answer":[{"name":"aonly.test.","type":1,"data":"9.9.9.9"}]}"#,
                    )
                } else {
                    ("500 Internal Server Error", "{}")
                };
                let resp = format!(
                    "HTTP/1.1 {}\r\nContent-Type: application/dns-json\r\nContent-Length: {}\r\n\r\n{}",
                    status,
                    body.len(),
                    body
                );
                tokio::io::AsyncWriteExt::write_all(&mut sock, resp.as_bytes())
                    .await
                    .unwrap();
                tokio::io::AsyncWriteExt::shutdown(&mut sock).await.unwrap();
            }
        });

        let mock_endpoint = format!("http://{addr}/dns-query");
        let res = resolve_with_endpoints("aonly.test", &[mock_endpoint]).await;
        server.abort();

        assert!(res.is_ok());
        let addrs = res.unwrap();
        assert_eq!(addrs.len(), 1);
        assert_eq!(addrs[0], "9.9.9.9".parse::<std::net::IpAddr>().unwrap());
    }

    /// 两个端点都不可达时总耗时 ≤ 4×2s（两端点 × 单查询2s超时，串行回退）。
    /// 用超时包裹验证，不必精确。
    #[tokio::test]
    async fn both_endpoints_unreachable_timeout_bound() {
        clear_cache();
        // 使用一个不存在的地址，连接会立即失败（Connection refused），总耗时应远小于 8s
        let start = Instant::now();
        let res = resolve_real_addrs("unreachable.invalid.test").await;
        let elapsed = start.elapsed();
        assert!(res.is_err());
        // 给足余量：连接被拒很快，即使有重试也应在 5s 内结束
        assert!(
            elapsed < Duration::from_secs(5),
            "total timeout should be bounded, got {:?}",
            elapsed
        );
    }
}
