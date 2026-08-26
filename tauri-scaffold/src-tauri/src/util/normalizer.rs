// src-tauri/src/util/normalizer.rs
//! 订阅归一化（Subscription Normalizer）
//!
//! 现代 Mihomo 订阅并不都是顶层 `proxies:`。不少订阅/配置长这样：
//!
//! ```yaml
//! proxy-providers:
//!   provider1:
//!     type: http
//!     url: https://example.com/nodes.yaml
//! ```
//!
//! 这类配置导入后若只按顶层 `proxies` 处理，会得到"UI 显示已激活、Mihomo 运行、
//! 但节点为 0"的假成功（对应 Issue #1）。本模块在**导入/刷新**时把任意订阅
//! 归一化成一个标准的"节点集"（`proxies:`），后续交给 ClashEdge 内置策略组。
//!
//! 安全边界保持不变：本模块只抽取**节点**（proxies），订阅自带的
//! proxy-groups / rules / hosts / sniffer / script / listeners 一律不进入运行时；
//! 远程 provider 的拉取复用 SSRF 防护的 `fetch::get_direct_first`；本地 file 型
//! provider 只允许解析应用数据目录内的相对安全路径，拒绝任意绝对路径读取。

use serde_yaml::Value;
use tauri::AppHandle;

use crate::util::error::{Error, Result};

/// 归一化结果：标准节点集 + 过程提示（跳过/失败的原因，供前端与日志反馈）。
pub struct NormalizedSubscription {
    pub proxies: Vec<Value>,
    pub warnings: Vec<String>,
}

/// 单次 provider 拉取的最大字节数（防御恶意超大响应）。
const MAX_PROVIDER_BYTES: u64 = 10 * 1024 * 1024;
/// 归一化后节点总数上限（与 profiles.rs 的 MAX_NODE_COUNT 对齐）。
const MAX_NODE_COUNT: usize = 1000;

/// 把订阅内容归一化为标准节点集。
///
/// `body` 为订阅原始 YAML（不含 `# subscribe-url:` 注释头；注释行本身无影响）。
///
/// 判定顺序：
/// 1. 顶层 `proxies` 非空 → 直接作为节点集（并顺带展开 `proxy-providers` 追加）；
/// 2. 仅 `proxy-providers` → 逐个安全展开（http 拉取 / inline 内联 / file 本地安全路径）；
/// 3. 两者都没有 → 空节点集（应用零节点兜底 DIRECT，不视为错误）。
pub async fn normalize_subscription(app: &AppHandle, body: &str) -> Result<NormalizedSubscription> {
    let value: Value = match serde_yaml::from_str(body) {
        Ok(v) => v,
        Err(e) => {
            return Err(Error::Subscription(format!(
                "Invalid subscription YAML: {}",
                e
            )))
        }
    };
    let Some(map) = value.as_mapping() else {
        return Err(Error::Subscription(
            "Subscription root must be a YAML mapping".to_string(),
        ));
    };

    let mut warnings: Vec<String> = Vec::new();
    let mut proxies: Vec<Value> = Vec::new();

    // 1) 顶层 proxies（原始节点，不改名）
    if let Some(seq) = map.get("proxies").and_then(|v| v.as_sequence()) {
        proxies.extend(seq.iter().cloned());
    }

    // 2) proxy-providers 展开（追加，节点名加 provider 前缀避免跨 provider 冲突）
    if let Some(providers) = map.get("proxy-providers").and_then(|v| v.as_mapping()) {
        for (name_val, prov_val) in providers {
            let name = name_val.as_str().unwrap_or("").to_string();
            let Some(prov_map) = prov_val.as_mapping() else {
                warnings.push(format!("provider '{}' is not a mapping; skipped", name));
                continue;
            };
            let ptype = prov_map
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("compatible")
                .to_ascii_lowercase();
            match ptype.as_str() {
                "http" | "compatible" => match expand_http_provider(app, &name, prov_map).await {
                    Ok(nodes) => append_prefixed(&mut proxies, &name, nodes),
                    Err(e) => warnings.push(format!(
                        "http provider '{}' could not be expanded: {}",
                        name, e
                    )),
                },
                "inline" => {
                    let nodes = prov_map
                        .get("proxies")
                        .and_then(|v| v.as_sequence())
                        .cloned()
                        .unwrap_or_default();
                    append_prefixed(&mut proxies, &name, nodes);
                }
                "file" => match expand_file_provider(app, &name, prov_map) {
                    Ok(nodes) => append_prefixed(&mut proxies, &name, nodes),
                    Err(e) => warnings.push(format!(
                        "file provider '{}' could not be expanded: {}",
                        name, e
                    )),
                },
                other => warnings.push(format!(
                    "provider '{}' has unsupported type '{}'; skipped",
                    name, other
                )),
            }
        }
    }

    if proxies.len() > MAX_NODE_COUNT {
        return Err(Error::Subscription(format!(
            "Normalized node count {} exceeds limit of {}",
            proxies.len(),
            MAX_NODE_COUNT
        )));
    }

    // 4) 明确反馈：订阅只声明了 proxy-providers，却一个节点都没展开出来
    //    （全部拉取/解析失败或类型不支持）→ 返回明确错误，避免"导入成功但零节点"的
    //    假成功。若存在顶层 proxies 或至少一个 provider 成功，则降级为警告继续。
    if proxies.is_empty() && map.contains_key("proxy-providers") && !warnings.is_empty() {
        return Err(Error::Subscription(format!(
            "订阅声明了 proxy-providers，但未能展开任何节点：{}",
            warnings.join("；")
        )));
    }

    // 3) 按名称去重（顶层与 provider 节点可能重名）
    proxies = dedupe_by_name(proxies);

    Ok(NormalizedSubscription { proxies, warnings })
}

/// 拉取 http 型 provider 的远端 YAML，抽取其顶层 `proxies`。
async fn expand_http_provider(
    app: &AppHandle,
    name: &str,
    prov_map: &serde_yaml::Mapping,
) -> Result<Vec<Value>> {
    let url = prov_map
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::Subscription(format!("http provider '{}' has no url", name)))?;

    // SSRF 防护与订阅拉取一致
    crate::util::fetch::validate_url(url).await?;

    let mut resp = crate::util::fetch::get_direct_first(app, url).await?;
    if !resp.status().is_success() {
        return Err(Error::Subscription(format!("HTTP {}", resp.status())));
    }
    if let Some(len) = resp.content_length() {
        if len > MAX_PROVIDER_BYTES {
            return Err(Error::Subscription(format!(
                "provider response exceeds {} bytes",
                MAX_PROVIDER_BYTES
            )));
        }
    }
    let mut bytes: Vec<u8> = Vec::new();
    while let Some(chunk) = resp.chunk().await? {
        bytes.extend_from_slice(&chunk);
        if bytes.len() as u64 > MAX_PROVIDER_BYTES {
            return Err(Error::Subscription(format!(
                "provider response exceeds {} bytes",
                MAX_PROVIDER_BYTES
            )));
        }
    }
    let text = String::from_utf8(bytes)
        .map_err(|e| Error::Subscription(format!("provider response not UTF-8: {}", e)))?;

    let doc: Value = serde_yaml::from_str(&text)
        .map_err(|e| Error::Subscription(format!("provider YAML invalid: {}", e)))?;
    Ok(doc
        .as_mapping()
        .and_then(|m| m.get("proxies"))
        .and_then(|v| v.as_sequence())
        .cloned()
        .unwrap_or_default())
}

/// 读取 file 型 provider：仅允许应用数据目录内的相对安全路径。
/// 拒绝绝对路径 / 含 `..` 的穿越路径 / 非 .yaml/.yml 扩展，防止订阅触发任意本地文件读取。
fn expand_file_provider(
    app: &AppHandle,
    name: &str,
    prov_map: &serde_yaml::Mapping,
) -> Result<Vec<Value>> {
    let path = prov_map
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::Subscription(format!("file provider '{}' has no path", name)))?;

    let p = std::path::Path::new(path);
    if p.is_absolute()
        || p.components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(Error::Subscription(format!(
            "file provider '{}' path is not a safe relative path",
            name
        )));
    }
    let ext_ok = p.extension().and_then(|e| e.to_str()).map(|e| {
        let e = e.to_ascii_lowercase();
        e == "yaml" || e == "yml"
    }) == Some(true);
    if !ext_ok {
        return Err(Error::Subscription(format!(
            "file provider '{}' path must be a .yaml/.yml file",
            name
        )));
    }

    let base = crate::util::paths::get_app_data_dir(app)
        .map_err(|e| Error::Subscription(format!("cannot resolve data dir: {}", e)))?;
    let full = base.join(p);
    // 再次确认解析后仍落在 base 内（防御符号链接/绝对拼接）
    if !full.starts_with(&base) {
        return Err(Error::Subscription(format!(
            "file provider '{}' escapes data dir",
            name
        )));
    }
    let text = std::fs::read_to_string(&full)
        .map_err(|e| Error::Subscription(format!("file provider '{}' unreadable: {}", name, e)))?;
    let doc: Value = serde_yaml::from_str(&text)
        .map_err(|e| Error::Subscription(format!("provider YAML invalid: {}", e)))?;
    Ok(doc
        .as_mapping()
        .and_then(|m| m.get("proxies"))
        .and_then(|v| v.as_sequence())
        .cloned()
        .unwrap_or_default())
}

/// 把 provider 节点追加进总集，节点名加 `{provider}-` 前缀（mihomo 惯例，防跨 provider 冲突）。
fn append_prefixed(out: &mut Vec<Value>, provider: &str, nodes: Vec<Value>) {
    for mut node in nodes {
        if !provider.is_empty() {
            if let Some(m) = node.as_mapping_mut() {
                if let Some(name) = m.get_mut("name") {
                    let orig = name.as_str().unwrap_or("");
                    *name = Value::from(format!("{}-{}", provider, orig));
                }
            }
        }
        out.push(node);
    }
}

/// 按 `name` 去重，保留首个出现。
fn dedupe_by_name(nodes: Vec<Value>) -> Vec<Value> {
    let mut seen = std::collections::HashSet::new();
    nodes
        .into_iter()
        .filter(|n| {
            let name = n
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            seen.insert(name)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedupe_keeps_first() {
        let node = |n: &str| -> Value {
            serde_yaml::from_str(&format!(
                "name: {}\ntype: ss\nserver: 1.1.1.1\nport: 8388\ncipher: aes-128-gcm\npassword: x\n",
                n
            ))
            .unwrap()
        };
        let out = dedupe_by_name(vec![node("a"), node("b"), node("a")]);
        let names: Vec<&str> = out
            .iter()
            .filter_map(|n| n.get("name").and_then(|v| v.as_str()))
            .collect();
        assert_eq!(names, vec!["a", "b"]);
    }

    #[test]
    fn prefix_provider_nodes() {
        let node = serde_yaml::from_str::<Value>(
            "name: n1\ntype: ss\nserver: 1.1.1.1\nport: 8388\ncipher: aes-128-gcm\npassword: x\n",
        )
        .unwrap();
        let mut out = Vec::new();
        append_prefixed(&mut out, "p1", vec![node]);
        assert_eq!(out[0].get("name").and_then(|v| v.as_str()), Some("p1-n1"));
    }
}
