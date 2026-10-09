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

use std::net::IpAddr;
use std::time::Duration;

use crate::util::error::{Error, Result};

/// DoH JSON 端点（IP 字面量，证书含 IP SAN）。依次尝试。
const DOH_ENDPOINTS: &[&str] = &["https://223.5.5.5/resolve", "https://1.1.1.1/dns-query"];
const DOH_TIMEOUT: Duration = Duration::from_secs(4);

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

/// 取 `host` 的真实 A + AAAA 地址。全部端点失败 → Err。
pub async fn resolve_real_addrs(host: &str) -> Result<Vec<IpAddr>> {
    let client = reqwest::Client::builder()
        .timeout(DOH_TIMEOUT)
        .no_proxy()
        .build()?;
    let mut last: Option<Error> = None;
    for ep in DOH_ENDPOINTS {
        let a = query(&client, ep, host, "A").await;
        let aaaa = query(&client, ep, host, "AAAA").await;
        match (a, aaaa) {
            (Ok(mut v4), Ok(v6)) => {
                v4.extend(v6);
                return Ok(v4);
            }
            // A 记录成功即可；AAAA 失败（部分端点不支持）不应否决结果
            (Ok(v4), Err(_)) if !v4.is_empty() => return Ok(v4),
            (Err(e), _) | (_, Err(e)) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| Error::Other("no DoH endpoint available".into())))
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
