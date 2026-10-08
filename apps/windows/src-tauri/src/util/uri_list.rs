// src-tauri/src/util/uri_list.rs
//! URI 列表 / Base64 订阅解码（审计 B7）
//!
//! 许多订阅服务端会按请求 UA 返回"每行一个分享链接"的文本（整体可能再做一层
//! Base64）。本模块把它们转换成 Mihomo 的 `proxies` 节点映射，之后走与 YAML 订阅
//! 完全相同的归一化 / 校验 / 注入内置分组流程。
//!
//! 支持：`ss://`、`vmess://`、`vless://`、`trojan://`、`hysteria2://`（`hy2://`）、
//! `tuic://`。无法识别或字段缺失的行记入 warnings，不影响其余节点。

use base64::{engine::general_purpose, Engine as _};
use serde_yaml::{Mapping, Value};

const SCHEMES: &[&str] = &[
    "ss://",
    "vmess://",
    "vless://",
    "trojan://",
    "hysteria2://",
    "hy2://",
    "tuic://",
];

/// 解析结果
pub struct UriListResult {
    pub proxies: Vec<Value>,
    pub warnings: Vec<String>,
}

fn is_share_line(line: &str) -> bool {
    let l = line.trim();
    SCHEMES
        .iter()
        .any(|s| l.to_ascii_lowercase().starts_with(s))
}

fn b64_decode(s: &str) -> Option<Vec<u8>> {
    let cleaned: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if cleaned.is_empty() {
        return None;
    }
    general_purpose::STANDARD
        .decode(&cleaned)
        .or_else(|_| general_purpose::STANDARD_NO_PAD.decode(&cleaned))
        .or_else(|_| general_purpose::URL_SAFE.decode(&cleaned))
        .or_else(|_| general_purpose::URL_SAFE_NO_PAD.decode(&cleaned))
        .ok()
}

/// 取得"每行一个链接"的明文：本身就是链接列表则原样返回，否则尝试整体 Base64 解码。
fn plain_body(text: &str) -> Option<String> {
    let t = text.trim().trim_start_matches('\u{feff}');
    if t.lines().any(is_share_line) {
        return Some(t.to_string());
    }
    let decoded = b64_decode(t)?;
    let s = String::from_utf8(decoded).ok()?;
    if s.lines().any(is_share_line) {
        Some(s)
    } else {
        None
    }
}

/// 文本是否看起来是 URI 列表（明文或 Base64 包裹）。
pub fn looks_like_uri_list(text: &str) -> bool {
    plain_body(text).is_some()
}

fn pct_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn s(v: &str) -> Value {
    Value::String(v.to_string())
}

fn put(m: &mut Mapping, k: &str, v: Value) {
    m.insert(Value::String(k.to_string()), v);
}

fn truthy(v: &str) -> bool {
    matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes")
}

fn csv(v: &str) -> Value {
    Value::Sequence(
        v.split(',')
            .map(|x| x.trim())
            .filter(|x| !x.is_empty())
            .map(s)
            .collect(),
    )
}

/// 解析一批分享链接。
pub fn parse(text: &str) -> UriListResult {
    let mut proxies: Vec<Value> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let Some(body) = plain_body(text) else {
        return UriListResult { proxies, warnings };
    };
    let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (idx, raw) in body.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if !is_share_line(line) {
            warnings.push(format!("line {}: unsupported scheme; skipped", idx + 1));
            continue;
        }
        match parse_line(line) {
            Ok(mut node) => {
                // 保证名称唯一（后续按名称注入分组 / 去重）
                let base = node
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("node")
                    .to_string();
                let mut name = base.clone();
                let mut n = 2;
                while !used.insert(name.clone()) {
                    name = format!("{} #{}", base, n);
                    n += 1;
                }
                if let Some(m) = node.as_mapping_mut() {
                    put(m, "name", s(&name));
                }
                proxies.push(node);
            }
            Err(e) => warnings.push(format!("line {}: {}", idx + 1, e)),
        }
    }
    UriListResult { proxies, warnings }
}

fn parse_line(line: &str) -> Result<Value, String> {
    let lower = line.to_ascii_lowercase();
    if lower.starts_with("ss://") {
        parse_ss(line)
    } else if lower.starts_with("vmess://") {
        parse_vmess(line)
    } else if lower.starts_with("vless://") {
        parse_vless(line)
    } else if lower.starts_with("trojan://") {
        parse_trojan(line)
    } else if lower.starts_with("hysteria2://") || lower.starts_with("hy2://") {
        parse_hy2(line)
    } else if lower.starts_with("tuic://") {
        parse_tuic(line)
    } else {
        Err("unsupported scheme".to_string())
    }
}

fn after_scheme(line: &str) -> &str {
    line.split_once("://").map(|x| x.1).unwrap_or("")
}

fn default_name(kind: &str, server: &str, port: u16) -> String {
    format!("{}-{}:{}", kind, server, port)
}

fn split_fragment(rest: &str) -> (&str, Option<String>) {
    match rest.split_once('#') {
        Some((a, f)) => (a, Some(pct_decode(f))),
        None => (rest, None),
    }
}

fn split_query(rest: &str) -> (&str, Vec<(String, String)>) {
    match rest.split_once('?') {
        Some((a, q)) => {
            let pairs = url_pairs(q);
            (a, pairs)
        }
        None => (rest, Vec::new()),
    }
}

fn url_pairs(q: &str) -> Vec<(String, String)> {
    q.split('&')
        .filter(|p| !p.is_empty())
        .map(|p| match p.split_once('=') {
            Some((k, v)) => (pct_decode(k), pct_decode(&v.replace('+', " "))),
            None => (pct_decode(p), String::new()),
        })
        .collect()
}

fn qget<'a>(q: &'a [(String, String)], key: &str) -> Option<&'a str> {
    q.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v.as_str())
        .filter(|v| !v.is_empty())
}

/// `host:port`（支持 `[v6]:port`）
fn split_host_port(hp: &str) -> Result<(String, u16), String> {
    let hp = hp.trim_end_matches('/');
    let (host, port) = if let Some(rest) = hp.strip_prefix('[') {
        let (h, p) = rest.split_once("]:").ok_or("invalid [host]:port")?;
        (h.to_string(), p)
    } else {
        let (h, p) = hp.rsplit_once(':').ok_or("missing port")?;
        (h.to_string(), p)
    };
    let port: u16 = p_u16(port)?;
    if host.is_empty() {
        return Err("missing host".to_string());
    }
    Ok((host, port))
}

fn p_u16(p: &str) -> Result<u16, String> {
    p.trim()
        .parse::<u16>()
        .ok()
        .filter(|p| *p != 0)
        .ok_or_else(|| format!("invalid port '{}'", p))
}

fn apply_transport(
    m: &mut Mapping,
    net: &str,
    host: Option<&str>,
    path: Option<&str>,
    svc: Option<&str>,
) {
    match net {
        "ws" | "websocket" => {
            put(m, "network", s("ws"));
            let mut o = Mapping::new();
            if let Some(p) = path {
                put(&mut o, "path", s(p));
            }
            if let Some(h) = host {
                let mut hd = Mapping::new();
                put(&mut hd, "Host", s(h));
                put(&mut o, "headers", Value::Mapping(hd));
            }
            if !o.is_empty() {
                put(m, "ws-opts", Value::Mapping(o));
            }
        }
        "grpc" => {
            put(m, "network", s("grpc"));
            let name = svc.or(path).unwrap_or("");
            if !name.is_empty() {
                let mut o = Mapping::new();
                put(&mut o, "grpc-service-name", s(name));
                put(m, "grpc-opts", Value::Mapping(o));
            }
        }
        "h2" | "http" => {
            put(m, "network", s("h2"));
            let mut o = Mapping::new();
            if let Some(h) = host {
                put(&mut o, "host", Value::Sequence(vec![s(h)]));
            }
            if let Some(p) = path {
                put(&mut o, "path", s(p));
            }
            if !o.is_empty() {
                put(m, "h2-opts", Value::Mapping(o));
            }
        }
        _ => {}
    }
}

fn parse_ss(line: &str) -> Result<Value, String> {
    let rest = after_scheme(line);
    let (rest, frag) = split_fragment(rest);
    let (rest, query) = split_query(rest);
    let rest = rest.trim_end_matches('/');

    let (method, password, server, port) = if let Some((userinfo, hp)) = rest.rsplit_once('@') {
        // SIP002：userinfo 为 base64(method:password) 或明文 method:password（可百分号编码）
        let (m, p) = match b64_decode(userinfo)
            .and_then(|b| String::from_utf8(b).ok())
            .filter(|t| t.contains(':'))
        {
            Some(t) => {
                let (m, p) = t.split_once(':').unwrap();
                (m.to_string(), p.to_string())
            }
            None => {
                let t = pct_decode(userinfo);
                let (m, p) = t.split_once(':').ok_or("invalid ss userinfo")?;
                (m.to_string(), p.to_string())
            }
        };
        let (h, port) = split_host_port(hp)?;
        (m, p, h, port)
    } else {
        // 旧格式：整体 base64(method:password@host:port)
        let t = b64_decode(rest)
            .and_then(|b| String::from_utf8(b).ok())
            .ok_or("invalid ss base64")?;
        let (userinfo, hp) = t.rsplit_once('@').ok_or("invalid ss payload")?;
        let (m, p) = userinfo.split_once(':').ok_or("invalid ss userinfo")?;
        let (h, port) = split_host_port(hp)?;
        (m.to_string(), p.to_string(), h, port)
    };

    let mut m = Mapping::new();
    put(
        &mut m,
        "name",
        s(&frag.unwrap_or_else(|| default_name("ss", &server, port))),
    );
    put(&mut m, "type", s("ss"));
    put(&mut m, "server", s(&server));
    put(&mut m, "port", Value::from(port));
    put(&mut m, "cipher", s(&method));
    put(&mut m, "password", s(&password));
    put(&mut m, "udp", Value::Bool(true));

    if let Some(plugin) = qget(&query, "plugin") {
        let mut parts = plugin.split(';');
        let pname = parts.next().unwrap_or("");
        let mut opts: Vec<(String, String)> = Vec::new();
        for kv in parts {
            match kv.split_once('=') {
                Some((k, v)) => opts.push((k.to_string(), v.to_string())),
                None => opts.push((kv.to_string(), "true".to_string())),
            }
        }
        let get = |k: &str| opts.iter().find(|(a, _)| a == k).map(|(_, v)| v.clone());
        match pname {
            "obfs-local" | "simple-obfs" => {
                put(&mut m, "plugin", s("obfs"));
                let mut o = Mapping::new();
                if let Some(v) = get("obfs") {
                    put(&mut o, "mode", s(&v));
                }
                if let Some(v) = get("obfs-host") {
                    put(&mut o, "host", s(&v));
                }
                put(&mut m, "plugin-opts", Value::Mapping(o));
            }
            "v2ray-plugin" => {
                put(&mut m, "plugin", s("v2ray-plugin"));
                let mut o = Mapping::new();
                put(
                    &mut o,
                    "mode",
                    s(&get("mode").unwrap_or_else(|| "websocket".into())),
                );
                if opts.iter().any(|(k, _)| k == "tls") {
                    put(&mut o, "tls", Value::Bool(true));
                }
                if let Some(v) = get("host") {
                    put(&mut o, "host", s(&v));
                }
                if let Some(v) = get("path") {
                    put(&mut o, "path", s(&v));
                }
                put(&mut m, "plugin-opts", Value::Mapping(o));
            }
            _ => {}
        }
    }
    Ok(Value::Mapping(m))
}

fn parse_vmess(line: &str) -> Result<Value, String> {
    let rest = after_scheme(line);
    let json_bytes = b64_decode(rest).ok_or("invalid vmess base64")?;
    let v: serde_json::Value =
        serde_json::from_slice(&json_bytes).map_err(|e| format!("invalid vmess json: {}", e))?;
    let get = |k: &str| -> Option<String> {
        v.get(k)
            .and_then(|x| match x {
                serde_json::Value::String(s) => Some(s.clone()),
                serde_json::Value::Number(n) => Some(n.to_string()),
                _ => None,
            })
            .filter(|s| !s.is_empty())
    };
    let server = get("add").ok_or("vmess missing add")?;
    let port = p_u16(&get("port").ok_or("vmess missing port")?)?;
    let uuid = get("id").ok_or("vmess missing id")?;
    let mut m = Mapping::new();
    put(
        &mut m,
        "name",
        s(&get("ps").unwrap_or_else(|| default_name("vmess", &server, port))),
    );
    put(&mut m, "type", s("vmess"));
    put(&mut m, "server", s(&server));
    put(&mut m, "port", Value::from(port));
    put(&mut m, "uuid", s(&uuid));
    put(
        &mut m,
        "alterId",
        Value::from(get("aid").and_then(|a| a.parse::<u64>().ok()).unwrap_or(0)),
    );
    put(
        &mut m,
        "cipher",
        s(&get("scy").unwrap_or_else(|| "auto".into())),
    );
    put(&mut m, "udp", Value::Bool(true));
    let tls = get("tls")
        .map(|t| t == "tls" || truthy(&t))
        .unwrap_or(false);
    if tls {
        put(&mut m, "tls", Value::Bool(true));
        if let Some(sni) = get("sni").or_else(|| get("host")) {
            put(&mut m, "servername", s(&sni));
        }
    }
    if let Some(fp) = get("fp") {
        put(&mut m, "client-fingerprint", s(&fp));
    }
    if let Some(alpn) = get("alpn") {
        put(&mut m, "alpn", csv(&alpn));
    }
    let net = get("net").unwrap_or_else(|| "tcp".into());
    apply_transport(
        &mut m,
        &net,
        get("host").as_deref(),
        get("path").as_deref(),
        None,
    );
    Ok(Value::Mapping(m))
}

struct UrlParts {
    user: String,
    password: Option<String>,
    server: String,
    port: u16,
    query: Vec<(String, String)>,
    name: Option<String>,
}

fn parse_authority(line: &str) -> Result<UrlParts, String> {
    let rest = after_scheme(line);
    let (rest, name) = split_fragment(rest);
    let (rest, query) = split_query(rest);
    let rest = rest.trim_end_matches('/');
    let (userinfo, hp) = rest.rsplit_once('@').ok_or("missing credentials")?;
    let (user, password) = match userinfo.split_once(':') {
        Some((u, p)) => (pct_decode(u), Some(pct_decode(p))),
        None => (pct_decode(userinfo), None),
    };
    let (server, port) = split_host_port(hp)?;
    if user.is_empty() {
        return Err("empty credentials".to_string());
    }
    Ok(UrlParts {
        user,
        password,
        server,
        port,
        query,
        name,
    })
}

fn tls_common(m: &mut Mapping, q: &[(String, String)], sni_key: &str) {
    if let Some(sni) = qget(q, sni_key).or_else(|| qget(q, "peer")) {
        put(m, "servername", s(sni));
    }
    if let Some(fp) = qget(q, "fp") {
        put(m, "client-fingerprint", s(fp));
    }
    if let Some(alpn) = qget(q, "alpn") {
        put(m, "alpn", csv(alpn));
    }
    if qget(q, "allowInsecure")
        .or_else(|| qget(q, "insecure"))
        .map(truthy)
        .unwrap_or(false)
    {
        put(m, "skip-cert-verify", Value::Bool(true));
    }
}

fn parse_vless(line: &str) -> Result<Value, String> {
    let u = parse_authority(line)?;
    let q = &u.query;
    let mut m = Mapping::new();
    put(
        &mut m,
        "name",
        s(&u.name
            .clone()
            .unwrap_or_else(|| default_name("vless", &u.server, u.port))),
    );
    put(&mut m, "type", s("vless"));
    put(&mut m, "server", s(&u.server));
    put(&mut m, "port", Value::from(u.port));
    put(&mut m, "uuid", s(&u.user));
    put(&mut m, "udp", Value::Bool(true));
    let security = qget(q, "security").unwrap_or("none").to_ascii_lowercase();
    if security == "tls" || security == "reality" {
        put(&mut m, "tls", Value::Bool(true));
        tls_common(&mut m, q, "sni");
    }
    if security == "reality" {
        let mut o = Mapping::new();
        if let Some(pbk) = qget(q, "pbk") {
            put(&mut o, "public-key", s(pbk));
        }
        if let Some(sid) = qget(q, "sid") {
            put(&mut o, "short-id", s(sid));
        }
        put(&mut m, "reality-opts", Value::Mapping(o));
    }
    if let Some(flow) = qget(q, "flow") {
        put(&mut m, "flow", s(flow));
    }
    let net = qget(q, "type").unwrap_or("tcp");
    apply_transport(
        &mut m,
        net,
        qget(q, "host"),
        qget(q, "path"),
        qget(q, "serviceName"),
    );
    Ok(Value::Mapping(m))
}

fn parse_trojan(line: &str) -> Result<Value, String> {
    let u = parse_authority(line)?;
    let q = &u.query;
    let password = match &u.password {
        Some(p) => format!("{}:{}", u.user, p),
        None => u.user.clone(),
    };
    let mut m = Mapping::new();
    put(
        &mut m,
        "name",
        s(&u.name
            .clone()
            .unwrap_or_else(|| default_name("trojan", &u.server, u.port))),
    );
    put(&mut m, "type", s("trojan"));
    put(&mut m, "server", s(&u.server));
    put(&mut m, "port", Value::from(u.port));
    put(&mut m, "password", s(&password));
    put(&mut m, "udp", Value::Bool(true));
    if let Some(sni) = qget(q, "sni").or_else(|| qget(q, "peer")) {
        put(&mut m, "sni", s(sni));
    }
    if let Some(fp) = qget(q, "fp") {
        put(&mut m, "client-fingerprint", s(fp));
    }
    if let Some(alpn) = qget(q, "alpn") {
        put(&mut m, "alpn", csv(alpn));
    }
    if qget(q, "allowInsecure")
        .or_else(|| qget(q, "insecure"))
        .map(truthy)
        .unwrap_or(false)
    {
        put(&mut m, "skip-cert-verify", Value::Bool(true));
    }
    let net = qget(q, "type").unwrap_or("tcp");
    apply_transport(
        &mut m,
        net,
        qget(q, "host"),
        qget(q, "path"),
        qget(q, "serviceName"),
    );
    Ok(Value::Mapping(m))
}

fn parse_hy2(line: &str) -> Result<Value, String> {
    let u = parse_authority(line)?;
    let q = &u.query;
    let password = match &u.password {
        Some(p) => format!("{}:{}", u.user, p),
        None => u.user.clone(),
    };
    let mut m = Mapping::new();
    put(
        &mut m,
        "name",
        s(&u.name
            .clone()
            .unwrap_or_else(|| default_name("hy2", &u.server, u.port))),
    );
    put(&mut m, "type", s("hysteria2"));
    put(&mut m, "server", s(&u.server));
    put(&mut m, "port", Value::from(u.port));
    put(&mut m, "password", s(&password));
    if let Some(sni) = qget(q, "sni") {
        put(&mut m, "sni", s(sni));
    }
    if qget(q, "insecure").map(truthy).unwrap_or(false) {
        put(&mut m, "skip-cert-verify", Value::Bool(true));
    }
    if let Some(o) = qget(q, "obfs") {
        put(&mut m, "obfs", s(o));
        if let Some(p) = qget(q, "obfs-password") {
            put(&mut m, "obfs-password", s(p));
        }
    }
    if let Some(alpn) = qget(q, "alpn") {
        put(&mut m, "alpn", csv(alpn));
    }
    if let Some(p) = qget(q, "mport") {
        put(&mut m, "ports", s(p));
    }
    Ok(Value::Mapping(m))
}

fn parse_tuic(line: &str) -> Result<Value, String> {
    let u = parse_authority(line)?;
    let q = &u.query;
    let password = u.password.clone().ok_or("tuic missing password")?;
    let mut m = Mapping::new();
    put(
        &mut m,
        "name",
        s(&u.name
            .clone()
            .unwrap_or_else(|| default_name("tuic", &u.server, u.port))),
    );
    put(&mut m, "type", s("tuic"));
    put(&mut m, "server", s(&u.server));
    put(&mut m, "port", Value::from(u.port));
    put(&mut m, "uuid", s(&u.user));
    put(&mut m, "password", s(&password));
    if let Some(sni) = qget(q, "sni") {
        put(&mut m, "sni", s(sni));
    }
    if let Some(cc) = qget(q, "congestion_control") {
        put(&mut m, "congestion-controller", s(cc));
    }
    if let Some(mode) = qget(q, "udp_relay_mode") {
        put(&mut m, "udp-relay-mode", s(mode));
    }
    if let Some(alpn) = qget(q, "alpn") {
        put(&mut m, "alpn", csv(alpn));
    }
    if qget(q, "allow_insecure")
        .or_else(|| qget(q, "insecure"))
        .map(truthy)
        .unwrap_or(false)
    {
        put(&mut m, "skip-cert-verify", Value::Bool(true));
    }
    Ok(Value::Mapping(m))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field<'a>(v: &'a Value, k: &str) -> Option<&'a Value> {
        v.get(k)
    }

    #[test]
    fn detects_plain_and_base64_lists() {
        assert!(looks_like_uri_list("trojan://pw@h.example:443#a\n"));
        let b64 = general_purpose::STANDARD
            .encode("trojan://pw@h.example:443#a\nss://YWVzLTEyOC1nY206cHc=@1.2.3.4:8388#b\n");
        assert!(looks_like_uri_list(&b64));
        assert!(!looks_like_uri_list("proxies:\n  - name: a\n"));
        assert!(!looks_like_uri_list("just some text"));
    }

    #[test]
    fn parses_ss_sip002_and_legacy() {
        // SIP002
        let b = general_purpose::URL_SAFE_NO_PAD.encode("aes-128-gcm:pa55");
        let r = parse(&format!("ss://{}@1.2.3.4:8388#%E9%A6%99%E6%B8%AF", b));
        assert_eq!(r.proxies.len(), 1, "{:?}", r.warnings);
        let n = &r.proxies[0];
        assert_eq!(field(n, "name").unwrap().as_str(), Some("香港"));
        assert_eq!(field(n, "cipher").unwrap().as_str(), Some("aes-128-gcm"));
        assert_eq!(field(n, "password").unwrap().as_str(), Some("pa55"));
        assert_eq!(field(n, "port").unwrap().as_u64(), Some(8388));
        // 旧格式
        let legacy = general_purpose::STANDARD.encode("aes-256-gcm:pw@example.com:443");
        let r = parse(&format!("ss://{}#old", legacy));
        assert_eq!(r.proxies.len(), 1, "{:?}", r.warnings);
        assert_eq!(
            field(&r.proxies[0], "server").unwrap().as_str(),
            Some("example.com")
        );
    }

    #[test]
    fn parses_ss_obfs_plugin() {
        let b = general_purpose::STANDARD.encode("aes-128-gcm:pw");
        let r = parse(&format!(
            "ss://{}@h.example:8388/?plugin=obfs-local%3Bobfs%3Dhttp%3Bobfs-host%3Dbing.com#p",
            b
        ));
        assert_eq!(r.proxies.len(), 1, "{:?}", r.warnings);
        let n = &r.proxies[0];
        assert_eq!(field(n, "plugin").unwrap().as_str(), Some("obfs"));
        let o = field(n, "plugin-opts").unwrap();
        assert_eq!(field(o, "mode").unwrap().as_str(), Some("http"));
        assert_eq!(field(o, "host").unwrap().as_str(), Some("bing.com"));
    }

    #[test]
    fn parses_vmess_ws_tls() {
        let json = r#"{"v":"2","ps":"vm","add":"v.example","port":"443","id":"11111111-1111-1111-1111-111111111111","aid":"0","net":"ws","host":"cdn.example","path":"/ws","tls":"tls","sni":"sni.example"}"#;
        let r = parse(&format!(
            "vmess://{}",
            general_purpose::STANDARD.encode(json)
        ));
        assert_eq!(r.proxies.len(), 1, "{:?}", r.warnings);
        let n = &r.proxies[0];
        assert_eq!(field(n, "type").unwrap().as_str(), Some("vmess"));
        assert_eq!(field(n, "network").unwrap().as_str(), Some("ws"));
        assert_eq!(
            field(n, "servername").unwrap().as_str(),
            Some("sni.example")
        );
        assert_eq!(
            field(field(n, "ws-opts").unwrap(), "path")
                .unwrap()
                .as_str(),
            Some("/ws")
        );
    }

    #[test]
    fn parses_vless_reality_and_trojan_and_hy2_and_tuic() {
        let text = "\
vless://22222222-2222-2222-2222-222222222222@r.example:443?security=reality&sni=www.example&fp=chrome&pbk=PUB&sid=ab&flow=xtls-rprx-vision&type=tcp#vless\n\
trojan://pw%40x@t.example:443?sni=t.example&type=grpc&serviceName=svc&allowInsecure=1#tj\n\
hysteria2://auth@h.example:8443?sni=h.example&insecure=1&obfs=salamander&obfs-password=op#hy\n\
tuic://33333333-3333-3333-3333-333333333333:pw@u.example:443?congestion_control=bbr&udp_relay_mode=native&alpn=h3#tu\n";
        let r = parse(text);
        assert_eq!(r.proxies.len(), 4, "{:?}", r.warnings);
        let vless = &r.proxies[0];
        assert_eq!(field(vless, "tls").unwrap().as_bool(), Some(true));
        assert_eq!(
            field(field(vless, "reality-opts").unwrap(), "public-key")
                .unwrap()
                .as_str(),
            Some("PUB")
        );
        assert_eq!(
            field(vless, "flow").unwrap().as_str(),
            Some("xtls-rprx-vision")
        );
        let tj = &r.proxies[1];
        assert_eq!(field(tj, "password").unwrap().as_str(), Some("pw@x"));
        assert_eq!(field(tj, "network").unwrap().as_str(), Some("grpc"));
        assert_eq!(field(tj, "skip-cert-verify").unwrap().as_bool(), Some(true));
        let hy = &r.proxies[2];
        assert_eq!(field(hy, "type").unwrap().as_str(), Some("hysteria2"));
        assert_eq!(field(hy, "obfs").unwrap().as_str(), Some("salamander"));
        let tu = &r.proxies[3];
        assert_eq!(
            field(tu, "congestion-controller").unwrap().as_str(),
            Some("bbr")
        );
    }

    #[test]
    fn duplicate_names_are_made_unique_and_bad_lines_warn() {
        let text = "trojan://p@a.example:443#same\ntrojan://p@b.example:443#same\ntrojan://p@c.example:0#bad\nftp://x\n";
        let r = parse(text);
        let names: Vec<&str> = r
            .proxies
            .iter()
            .filter_map(|p| p.get("name").and_then(|n| n.as_str()))
            .collect();
        assert_eq!(names, vec!["same", "same #2"]);
        assert_eq!(r.warnings.len(), 2, "{:?}", r.warnings);
    }

    #[test]
    fn pct_decode_handles_edges() {
        assert_eq!(pct_decode("a%20b"), "a b");
        assert_eq!(pct_decode("%zz"), "%zz");
        assert_eq!(pct_decode("100%"), "100%");
        assert_eq!(pct_decode("%E9%A6%99"), "香");
    }
}
