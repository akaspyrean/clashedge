// src-tauri/src/commands/profiles/validate.rs
//! 订阅内容校验：资源限制（大小/节点数/名称长度/字段长度）与
//! 按协议的必要字段校验（防止字段缺失导致 mihomo 启动失败）。

use crate::util::error::{Error, Result};

// 订阅资源限制
/// 最大 YAML 内容大小（10 MB 文本）
const MAX_YAML_CONTENT_BYTES: u64 = 10 * 1024 * 1024;
/// 最大节点数量
const MAX_NODE_COUNT: usize = 1000;
/// 最大节点名称长度
const MAX_NODE_NAME_LENGTH: usize = 100;
/// 任意字段值的最大长度（防止异常超长字段）
const MAX_FIELD_VALUE_LENGTH: usize = 5000;

/// 校验订阅内容是否符合资源限制。
/// 在写入磁盘前调用，防止恶意超大/超长订阅导致资源耗尽或 UI 异常。
pub(super) fn validate_subscription_content(text: &str) -> Result<()> {
    if text.len() > MAX_YAML_CONTENT_BYTES as usize {
        return Err(Error::Subscription(format!(
            "Subscription content exceeds {} bytes limit",
            MAX_YAML_CONTENT_BYTES
        )));
    }
    let value: serde_yaml::Value = serde_yaml::from_str(text)
        .map_err(|e| Error::Subscription(format!("Invalid YAML: {}", e)))?;
    // 校验节点
    if let Some(proxies) = value.get("proxies").and_then(|v| v.as_sequence()) {
        if proxies.len() > MAX_NODE_COUNT {
            return Err(Error::Subscription(format!(
                "Node count {} exceeds limit of {}",
                proxies.len(),
                MAX_NODE_COUNT
            )));
        }
        for (i, node) in proxies.iter().enumerate() {
            if let Some(name) = node.get("name").and_then(|n| n.as_str()) {
                if name.len() > MAX_NODE_NAME_LENGTH {
                    return Err(Error::Subscription(format!(
                        "Node #{} name length {} exceeds limit of {}",
                        i + 1,
                        name.len(),
                        MAX_NODE_NAME_LENGTH
                    )));
                }
            }
            // 按协议校验必要字段，不使用统一极小字段白名单。
            // 保证 VLESS/Reality/Trojan/Hysteria2/TUIC 等协议不被破坏。
            validate_node_protocol(node, i + 1)?;
            // 校验所有字段值长度
            if let Some(map) = node.as_mapping() {
                for (_, v) in map {
                    if let Some(s) = v.as_str() {
                        if s.len() > MAX_FIELD_VALUE_LENGTH {
                            return Err(Error::Subscription(format!(
                                "Node #{} contains a field value exceeding {} bytes",
                                i + 1,
                                MAX_FIELD_VALUE_LENGTH
                            )));
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// 按协议校验代理节点必要字段。
/// 每种协议有各自的必需字段，统一校验可防止字段缺失导致 mihomo 启动失败。
/// 不使用统一的极小字段白名单——保证 VLESS/Reality/Trojan/Hysteria2/TUIC
/// 等现有协议能力不被破坏。
fn validate_node_protocol(node: &serde_yaml::Value, index: usize) -> Result<()> {
    let Some(map) = node.as_mapping() else {
        return Err(Error::Subscription(format!(
            "Node #{} is not a mapping",
            index
        )));
    };
    let get_str = |key: &str| {
        map.get(serde_yaml::Value::String(key.to_string()))
            .and_then(|v| v.as_str())
    };
    let has = |key: &str| map.contains_key(serde_yaml::Value::String(key.to_string()));
    let protocol = get_str("type").unwrap_or("unknown");
    let name_raw = get_str("name").unwrap_or("");
    // 通用字段：所有节点都必须有非空的 name / type / server。
    // 缺失 name 会让 Normalizer 的 dedupe_by_name 把多个无名单节点折叠成一个
    // （name="" 全部去重只剩一个），且运行时靠 name 注入内置叶子组——必须显式
    // 拒绝，不做静默修复。
    if !has("name") || name_raw.trim().is_empty() {
        return Err(Error::Subscription(format!(
            "Node #{} ({}) is missing required non-empty field 'name'",
            index, protocol
        )));
    }
    let name = name_raw;
    if !has("type") || protocol.trim().is_empty() {
        return Err(Error::Subscription(format!(
            "Node #{} is missing required field 'type'",
            index
        )));
    }
    if !has("server") || get_str("server").unwrap_or("").trim().is_empty() {
        return Err(Error::Subscription(format!(
            "Node #{} ('{}', {}) is missing required field 'server'",
            index, name, protocol
        )));
    }
    // 按协议校验
    match protocol {
        "ss" | "shadowsocks" => {
            if !has("port") || !has("cipher") || !has("password") {
                return Err(Error::Subscription(format!(
                    "Node #{} ('{}', {}) requires port, cipher, password",
                    index, name, protocol
                )));
            }
        }
        "ssr" => {
            if !has("port")
                || !has("cipher")
                || !has("password")
                || !has("protocol")
                || !has("obfs")
            {
                return Err(Error::Subscription(format!(
                    "Node #{} ('{}', {}) requires port, cipher, password, protocol, obfs",
                    index, name, protocol
                )));
            }
        }
        "vmess" => {
            if !has("port") || !has("uuid") || !has("cipher") {
                return Err(Error::Subscription(format!(
                    "Node #{} ('{}', {}) requires port, uuid, cipher (alterId optional)",
                    index, name, protocol
                )));
            }
        }
        "trojan" => {
            if !has("port") || !has("password") {
                return Err(Error::Subscription(format!(
                    "Node #{} ('{}', {}) requires port, password",
                    index, name, protocol
                )));
            }
        }
        "vless" => {
            // VLESS 需要 port、uuid；flow、reality-opts 等为可选（Reality 场景）
            if !has("port") || !has("uuid") {
                return Err(Error::Subscription(format!(
                    "Node #{} ('{}', {}) requires port, uuid",
                    index, name, protocol
                )));
            }
        }
        "hysteria2" | "hy2" => {
            if !has("port") || !has("password") {
                return Err(Error::Subscription(format!(
                    "Node #{} ('{}', {}) requires port, password",
                    index, name, protocol
                )));
            }
        }
        "hysteria" => {
            if !has("port") || !has("auth_str") {
                return Err(Error::Subscription(format!(
                    "Node #{} ('{}', {}) requires port, auth_str",
                    index, name, protocol
                )));
            }
        }
        "tuic" => {
            if !has("port") || !has("token") || !has("congestion_control") {
                return Err(Error::Subscription(format!(
                    "Node #{} ('{}', {}) requires port, token, congestion_control",
                    index, name, protocol
                )));
            }
        }
        "http" | "https" | "socks5" if !has("port") => {
            return Err(Error::Subscription(format!(
                "Node #{} ('{}', {}) requires port",
                index, name, protocol
            )));
        }
        _ => {
            // 未知协议：仅要求 name 和 server（已校验），不阻断导入
            // 以便支持未来 mihomo 新增协议
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_requires_non_empty_name_type_server() {
        // 缺失 name：运行时靠 name 注入内置叶子组，且 Normalizer 的 dedupe_by_name
        // 会把多个无名单节点折叠成一个——必须显式拒绝，不做静默修复。
        let no_name: serde_yaml::Value = serde_yaml::from_str(
            "type: ss\nserver: 1.1.1.1\nport: 8388\ncipher: aes-128-gcm\npassword: x\n",
        )
        .unwrap();
        assert!(
            validate_node_protocol(&no_name, 1).is_err(),
            "missing name rejected"
        );

        // 缺失 type
        let no_type: serde_yaml::Value = serde_yaml::from_str(
            "name: n\nserver: 1.1.1.1\nport: 8388\ncipher: aes-128-gcm\npassword: x\n",
        )
        .unwrap();
        assert!(
            validate_node_protocol(&no_type, 2).is_err(),
            "missing type rejected"
        );

        // 缺失 server
        let no_server: serde_yaml::Value = serde_yaml::from_str(
            "name: n\ntype: ss\nport: 8388\ncipher: aes-128-gcm\npassword: x\n",
        )
        .unwrap();
        assert!(
            validate_node_protocol(&no_server, 3).is_err(),
            "missing server rejected"
        );

        // 合法节点通过
        let ok: serde_yaml::Value = serde_yaml::from_str(
            "name: n\ntype: ss\nserver: 1.1.1.1\nport: 8388\ncipher: aes-128-gcm\npassword: x\n",
        )
        .unwrap();
        assert!(
            validate_node_protocol(&ok, 4).is_ok(),
            "valid node accepted"
        );
    }

    /// 订阅兼容性 fixtures：合法 top-level proxies 通过。
    #[test]
    fn subscription_fixture_valid_top_level_proxies() {
        let content = r#"
proxies:
  - name: S1
    type: ss
    server: 1.1.1.1
    port: 8388
    cipher: aes-128-gcm
    password: a
  - name: S2
    type: vless
    server: 2.2.2.2
    port: 443
    uuid: xxx
"#;
        assert!(validate_subscription_content(content).is_ok());
    }

    /// 订阅兼容性 fixtures：非法 YAML 被拒绝。
    #[test]
    fn subscription_fixture_bad_yaml_rejected() {
        assert!(validate_subscription_content(
            "proxies:\n  - name: x\n    type: ss\n   server: [unclosed"
        )
        .is_err());
    }

    /// 订阅兼容性 fixtures：节点数超限被拒绝（资源预算防滥用）。
    #[test]
    fn subscription_fixture_too_many_nodes_rejected() {
        let mut s = String::from("proxies:\n");
        for i in 0..=MAX_NODE_COUNT {
            s.push_str(&format!(
                "  - name: N{}\n    type: ss\n    server: 1.1.1.1\n    port: 8388\n    cipher: aes-128-gcm\n    password: x\n",
                i
            ));
        }
        assert!(
            validate_subscription_content(&s).is_err(),
            "node count above limit rejected"
        );
    }

    /// 订阅兼容性 fixtures：空 proxies 允许（应用零节点兜底 DIRECT），非空注入验证。
    #[test]
    fn subscription_fixture_empty_proxies_ok_but_nonempty_provider_shaped() {
        // 空 proxies 是合法输入（应用兜底 DIRECT），不应被拒绝
        assert!(validate_subscription_content("proxies: []\n").is_ok());
        // 缺失字段的节点即使带 proxies 也被拒
        assert!(
            validate_subscription_content(
                "proxies:\n  - type: ss\n    server: 1.1.1.1\n    port: 8388\n    cipher: aes-128-gcm\n    password: x\n"
            )
            .is_err(),
            "node missing name rejected"
        );
    }
}
