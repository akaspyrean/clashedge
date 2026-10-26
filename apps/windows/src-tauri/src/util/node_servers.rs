// src-tauri/src/util/node_servers.rs
//! 节点名 → 服务器地址（用于 GeoIP 国家推断）。
//!
//! 订阅里的节点名经常是匿名主机名（例如 Just My Socks 的
//! `JMS-1416430@c37s1...`），名称本身不含国家信息。此时只能拿节点真实
//! `server` 的 IP 查本地 `Country.mmdb`。
//!
//! 数据来源是 mihomo 实际加载的 `runtime-config.yaml`（由
//! `core::config::build_runtime_config` 生成，`proxy-providers` 已展开为
//! 顶层 `proxies`），因此这里看到的名字与控制器 `/proxies` 返回的完全一致。
//!
//! 结果按文件 mtime 缓存：运行时配置在订阅刷新 / 节点切换后会重写，
//! 下一次查询自动重建，不会读到旧节点。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde_norway::Value;

type Cached = (PathBuf, SystemTime, HashMap<String, String>);

static CACHE: Lazy<Mutex<Option<Cached>>> = Lazy::new(|| Mutex::new(None));

/// 读取 `name -> server`。文件缺失 / 解析失败时返回空表（调用方静默回退）。
pub fn load(path: &Path) -> HashMap<String, String> {
    let Ok(meta) = std::fs::metadata(path) else {
        return HashMap::new();
    };
    let Ok(mtime) = meta.modified() else {
        return HashMap::new();
    };

    let mut guard = CACHE.lock();
    if let Some((cached_path, cached_mtime, map)) = guard.as_ref() {
        if cached_path == path && *cached_mtime == mtime {
            return map.clone();
        }
    }

    let Ok(text) = std::fs::read_to_string(path) else {
        return HashMap::new();
    };
    let map = parse(&text);
    *guard = Some((path.to_path_buf(), mtime, map.clone()));
    map
}

/// 从 runtime-config YAML 中抽取 `proxies[].name -> proxies[].server`。
fn parse(text: &str) -> HashMap<String, String> {
    let Ok(Value::Mapping(root)) = serde_norway::from_str::<Value>(text) else {
        return HashMap::new();
    };
    let Some(Value::Sequence(proxies)) = root.get(Value::String("proxies".into())) else {
        return HashMap::new();
    };

    let mut map = HashMap::new();
    for proxy in proxies {
        let Some(m) = proxy.as_mapping() else {
            continue;
        };
        let name = m
            .get(Value::String("name".into()))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let server = m
            .get(Value::String("server".into()))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty());
        if let (Some(name), Some(server)) = (name, server) {
            map.insert(name.to_string(), server.to_string());
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn extracts_name_to_server_and_skips_incomplete_entries() {
        let yaml = r#"
mixed-port: 7890
proxies:
  - name: "JMS-1@c37s1"
    type: ss
    server: c37s1.portablesubscribe.com
  - name: "JP-02"
    type: vmess
    server: 203.0.113.10
  - name: "missing-server"
    type: ss
  - name: ""
    server: ignored.example.com
"#;
        let map = parse(yaml);
        assert_eq!(map.len(), 2);
        assert_eq!(
            map.get("JMS-1@c37s1").map(String::as_str),
            Some("c37s1.portablesubscribe.com")
        );
        assert_eq!(map.get("JP-02").map(String::as_str), Some("203.0.113.10"));
    }

    #[test]
    fn missing_or_invalid_yaml_is_empty() {
        assert!(parse("").is_empty());
        assert!(parse("not: [valid").is_empty());
        assert!(parse("proxies: []\n").is_empty());
    }
}
