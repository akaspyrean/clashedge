// src-tauri/src/config/migration.rs
//! 配置版本迁移（P0-1 重构）
//!
//! 旧实现的问题：
//! - `migrate_mixed_format` / `migrate_from_yaml_string` 是空壳（返回"已迁移"
//!   但什么都没做）；
//! - `is_new_format` 恒为 true；
//! - 迁移直接写盘，失败路径会把用户配置静默覆盖成默认值。
//!
//! 新策略（与 docs/AUDIT-0.8.7.md P0-1 一致）：
//! - 迁移**只在内存中**完成：旧内容 → 识别已知字段 → 合并到默认结构 → 校验；
//! - 落盘交给调用方的下一次显式保存；调用方负责在迁移前备份原文件；
//! - 任何一步失败都返回 Err，绝不产生"看起来成功实际是默认值"的结果。

use crate::config::model::Config;
use crate::util::error::{Error, Result};

/// 把旧格式 / 部分损坏的配置文本迁移为新版 `Config`（仅内存，不落盘）。
///
/// 流程：
/// 1. 解析为通用 YAML 值（失败 → Err，由调用方进入降级/修复状态）；
/// 2. 根节点不是 mapping → Err；
/// 3. 能直接按新结构反序列化 → 原样返回（无需迁移）;
/// 4. 否则把可识别的旧字段（kebab-case / snake_case 双写法、顶层平铺或
///    分段嵌套）逐个合并到默认配置上。
pub fn migrate_content(content: &str) -> Result<Config> {
    let value: serde_yaml::Value = serde_yaml::from_str(content)
        .map_err(|e| Error::ConfigParse(format!("not valid YAML: {}", e)))?;
    if !value.is_mapping() {
        return Err(Error::ConfigParse(
            "config root must be a YAML mapping".to_string(),
        ));
    }
    // 直接按新结构（flatten 布局）解析；失败则走字段级宽松合并
    let mut config = match serde_yaml::from_value::<Config>(value.clone()) {
        Ok(config) => config,
        Err(_) => merge_legacy_value(&value),
    };
    // 兼容嵌套 `proxy:` 段的历史布局：controller 字段在子映射里时，
    // flatten 解析会把它丢进 extra，需要显式提取
    merge_nested_proxy_section(&value, &mut config);
    // 脱敏占位符不是真实密钥：清空后由 init/set_config 的密钥兜底逻辑轮换
    if config.proxy.secret == crate::config::model::SECRET_REDACTED {
        config.proxy.secret = String::new();
    }
    Ok(config)
}

/// 从顶层 `proxy:` 子映射中提取 external-controller / secret，
/// 仅当顶层没有对应字段时才补齐（顶层优先）。
fn merge_nested_proxy_section(value: &serde_yaml::Value, config: &mut Config) {
    let Some(map) = value.as_mapping() else {
        return;
    };
    if field(map, "external-controller").is_none() {
        if let Some(v) = sub_mapping(map, "proxy")
            .and_then(|m| as_string(field(m, "external-controller")))
            .filter(|s| !s.is_empty())
        {
            config.proxy.external_controller = v;
        }
    }
    if field(map, "secret").is_none() {
        if let Some(v) = sub_mapping(map, "proxy")
            .and_then(|m| as_string(field(m, "secret")))
            .filter(|s| !s.is_empty() && s != crate::config::model::SECRET_REDACTED)
        {
            config.proxy.secret = v;
        }
    }
}

/// 按 key 的多种历史写法取字段：kebab-case 与 snake_case 等价。
fn field<'a>(map: &'a serde_yaml::Mapping, name: &str) -> Option<&'a serde_yaml::Value> {
    let kebab = serde_yaml::Value::String(name.to_string());
    if let Some(v) = map.get(&kebab) {
        return Some(v);
    }
    let snake = serde_yaml::Value::String(name.replace('-', "_"));
    map.get(&snake)
}

fn as_bool(v: Option<&serde_yaml::Value>) -> Option<bool> {
    v.and_then(|v| v.as_bool())
}

fn as_string(v: Option<&serde_yaml::Value>) -> Option<String> {
    v.and_then(|v| v.as_str()).map(|s| s.to_string())
}

fn as_u64(v: Option<&serde_yaml::Value>) -> Option<u64> {
    v.and_then(|v| v.as_u64())
}

fn as_u16(v: Option<&serde_yaml::Value>) -> Option<u16> {
    v.and_then(|v| v.as_u64())
        .and_then(|n| u16::try_from(n).ok())
}

fn as_string_list(v: Option<&serde_yaml::Value>) -> Option<Vec<String>> {
    v.and_then(|v| v.as_sequence()).map(|seq| {
        seq.iter()
            .filter_map(|item| item.as_str().map(|s| s.to_string()))
            .collect()
    })
}

/// 子映射：顶层 `tun:` / `dns:` 等
fn sub_mapping<'a>(map: &'a serde_yaml::Mapping, name: &str) -> Option<&'a serde_yaml::Mapping> {
    field(map, name).and_then(|v| v.as_mapping())
}

/// 把可识别的旧字段合并到默认配置上（未出现的字段保持默认值）。
///
/// 兼容两种历史布局：
/// - 0.8.x 平铺格式：external-controller / secret 直接在顶层；
/// - 分段格式：controller 字段位于 `proxy:` 段下。
fn merge_legacy_value(value: &serde_yaml::Value) -> Config {
    let mut config = Config::default();
    let Some(map) = value.as_mapping() else {
        return config;
    };

    // --- general ---
    let general = &mut config.general;
    if let Some(v) = as_u16(field(map, "mixed-port")) {
        general.mixed_port = v;
    }
    if let Some(v) = as_bool(field(map, "allow-lan")) {
        general.allow_lan = v;
    }
    if let Some(v) = as_string(field(map, "log-level")).filter(|s| !s.is_empty()) {
        general.log_level = v;
    }
    if let Some(v) = as_bool(field(map, "ipv6")) {
        general.ipv6 = v;
    }
    if let Some(v) = field(map, "geodata-mode").cloned().filter(|v| !v.is_null()) {
        general.geodata_mode = v;
    }
    if let Some(v) = as_bool(field(map, "geo-auto-update")) {
        general.geo_auto_update = v;
    }
    if let Some(v) = as_string(field(map, "find-process-mode")).filter(|s| !s.is_empty()) {
        general.find_process_mode = v;
    }
    // mihomo 顶层键为 `mode`，历史写法 `proxy-mode`
    if let Some(v) =
        as_string(field(map, "mode").or_else(|| field(map, "proxy-mode"))).filter(|s| !s.is_empty())
    {
        general.proxy_mode = v;
    }
    if let Some(v) = as_string(field(map, "profile")) {
        general.profile = v;
    }

    // --- proxy（控制器）：顶层平铺或 `proxy:` 段 ---
    let controller_section = sub_mapping(map, "proxy");
    if let Some(v) = as_string(
        field(map, "external-controller")
            .or_else(|| controller_section.and_then(|m| field(m, "external-controller"))),
    )
    .filter(|s| !s.is_empty())
    {
        config.proxy.external_controller = v;
    }
    if let Some(v) = as_string(
        field(map, "secret").or_else(|| controller_section.and_then(|m| field(m, "secret"))),
    ) {
        if !v.is_empty() && v != crate::config::model::SECRET_REDACTED {
            config.proxy.secret = v;
        }
    }

    // --- tun ---
    if let Some(tun) = sub_mapping(map, "tun") {
        if let Some(v) = as_bool(field(tun, "enable")) {
            config.tun.enable = v;
        }
        if let Some(v) = as_string(field(tun, "stack")).filter(|s| !s.is_empty()) {
            config.tun.stack = v;
        }
        if let Some(v) = as_bool(field(tun, "auto-route")) {
            config.tun.auto_route = v;
        }
        if let Some(v) = as_bool(field(tun, "auto-detect-interface")) {
            config.tun.auto_detect_interface = v;
        }
        if let Some(v) = as_string(field(tun, "interface-name")).filter(|s| !s.is_empty()) {
            config.tun.interface_name = Some(v);
        }
        if let Some(v) = as_string_list(field(tun, "dns-hijack")) {
            if !v.is_empty() {
                config.tun.dns_hijack = v;
            }
        }
    }

    // --- dns ---
    if let Some(dns) = sub_mapping(map, "dns") {
        if let Some(v) = as_bool(field(dns, "enable")) {
            config.dns.enable = v;
        }
        if let Some(v) = as_string(field(dns, "listen")).filter(|s| !s.is_empty()) {
            config.dns.listen = v;
        }
        if let Some(v) = as_string(field(dns, "enhanced-mode")).filter(|s| !s.is_empty()) {
            config.dns.enhanced_mode = v;
        }
        if let Some(v) = as_string(field(dns, "fake-ip-range")).filter(|s| !s.is_empty()) {
            config.dns.fake_ip_range = v;
        }
        if let Some(v) = as_string_list(field(dns, "fake-ip-filter")) {
            if !v.is_empty() {
                config.dns.fake_ip_filter = v;
            }
        }
        if let Some(v) = as_string_list(field(dns, "default-nameserver")) {
            if !v.is_empty() {
                config.dns.default_nameserver = v;
            }
        }
        if let Some(v) = as_string_list(field(dns, "nameserver")) {
            if !v.is_empty() {
                config.dns.nameserver = v;
            }
        }
        if let Some(v) = as_string_list(field(dns, "proxy-server-nameserver")) {
            if !v.is_empty() {
                config.dns.proxy_server_nameserver = v;
            }
        }
    }

    // --- advanced ---
    if let Some(advanced) = sub_mapping(map, "advanced") {
        if let Some(v) = as_bool(field(advanced, "disable-commit-animation")) {
            config.advanced.disable_commit_animation = v;
        }
        if let Some(v) = as_string(field(advanced, "log-format")).filter(|s| !s.is_empty()) {
            config.advanced.log_format = v;
        }
        if let Some(v) = as_bool(field(advanced, "explicit-proxy")) {
            config.advanced.explicit_proxy = v;
        }
        if let Some(v) = as_u64(field(advanced, "connect-timeout")) {
            config.advanced.connect_timeout = v;
        }
        if let Some(v) = as_u64(field(advanced, "read-timeout")) {
            config.advanced.read_timeout = v;
        }
        if let Some(v) = as_u64(field(advanced, "write-timeout")) {
            config.advanced.write_timeout = v;
        }
        if let Some(v) = as_string(field(advanced, "geoip-url")) {
            config.advanced.geoip_url = v;
        }
        if let Some(v) = as_string(field(advanced, "geosite-url")) {
            config.advanced.geosite_url = v;
        }
        if let Some(v) = as_string(field(advanced, "geox-url")) {
            config.advanced.geox_url = v;
        }
    }

    // --- profiles ---
    if let Some(profiles) = sub_mapping(map, "profiles") {
        let p = &mut config.profiles;
        if let Some(v) = as_string_list(field(profiles, "proxies")) {
            p.proxies = v;
        }
        if let Some(v) = as_string(field(profiles, "default-profile")).filter(|s| !s.is_empty()) {
            p.default_profile = v;
        }
        if let Some(v) = as_string(field(profiles, "auto-group")).filter(|s| !s.is_empty()) {
            p.auto_group = v;
        }
        if let Some(v) = as_string(field(profiles, "manual-group")).filter(|s| !s.is_empty()) {
            p.manual_group = v;
        }
        if let Some(v) = as_string(field(profiles, "media-group")).filter(|s| !s.is_empty()) {
            p.media_group = v;
        }
        if let Some(v) = as_string(field(profiles, "ai-group")).filter(|s| !s.is_empty()) {
            p.ai_group = v;
        }
    }

    // --- mixin ---
    if let Some(v) = as_bool(field(map, "mixin-enabled")) {
        config.mixin_enabled = v;
    }

    config
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 旧版平铺格式：能识别全部已知字段并合并到默认结构上
    #[test]
    fn migrates_legacy_flat_layout() {
        let yaml = r#"
mixed-port: 7897
allow-lan: true
log-level: warning
find-process-mode: strict
proxy-mode: global
external-controller: 127.0.0.1:9091
secret: mysecret

dns:
  enable: false
  listen: 127.0.0.1:1053

tun:
  enable: true
  stack: gvisor
"#;
        let config = migrate_content(yaml).unwrap();
        assert_eq!(config.general.mixed_port, 7897);
        assert!(config.general.allow_lan);
        assert_eq!(config.general.log_level, "warning");
        assert_eq!(config.general.find_process_mode, "strict");
        assert_eq!(config.general.proxy_mode, "global");
        assert_eq!(config.proxy.external_controller, "127.0.0.1:9091");
        assert_eq!(config.proxy.secret, "mysecret");
        assert!(!config.dns.enable);
        assert_eq!(config.dns.listen, "127.0.0.1:1053");
        assert!(config.tun.enable);
        assert_eq!(config.tun.stack, "gvisor");
        // 未出现字段保持默认（DNS nameserver 有内置默认值）
        assert!(!config.dns.nameserver.is_empty());
    }

    /// 分段布局（controller 在 proxy: 段下）同样能识别
    #[test]
    fn migrates_sectioned_layout() {
        let yaml = r#"
mixed-port: 7890
proxy:
  external-controller: 127.0.0.1:9090
  secret: section-secret
"#;
        let config = migrate_content(yaml).unwrap();
        assert_eq!(config.proxy.external_controller, "127.0.0.1:9090");
        assert_eq!(config.proxy.secret, "section-secret");
    }

    /// 已经是新格式的配置原样通过（不丢字段）
    #[test]
    fn passes_new_format_through() {
        let mut config = Config::default();
        config.general.mixed_port = 7899;
        config.general.allow_lan = true;
        let yaml = serde_yaml::to_string(&config).unwrap();
        let migrated = migrate_content(&yaml).unwrap();
        assert_eq!(migrated.general.mixed_port, 7899);
        assert!(migrated.general.allow_lan);
    }

    /// H：老配置缺 `dns-hijack` 字段时能正常加载（默认补 any:53 + tcp://any:53），
    /// 已有合法 dns-hijack 的配置则原样保留，不再被默认覆盖。
    #[test]
    fn loads_legacy_tun_with_or_without_dns_hijack() {
        // 老配置缺 dns-hijack → 默认补全
        let yaml = "mixed-port: 7890\ntun:\n  enable: true\n  stack: gvisor\n";
        let config = migrate_content(yaml).unwrap();
        assert_eq!(config.tun.stack, "gvisor");
        assert_eq!(
            config.tun.dns_hijack,
            vec!["any:53".to_string(), "tcp://any:53".to_string()],
            "missing dns-hijack must default"
        );

        // 已有合法 dns-hijack → 保留
        let yaml = "mixed-port: 7890\ntun:\n  enable: true\n  stack: mixed\n  dns-hijack:\n    - 1.1.1.1:53\n";
        let config = migrate_content(yaml).unwrap();
        assert_eq!(
            config.tun.dns_hijack,
            vec!["1.1.1.1:53".to_string()],
            "existing dns-hijack must be preserved"
        );
    }

    /// 非 YAML / 根节点非 mapping → 明确报错（绝不返回默认配置冒充成功）
    #[test]
    fn rejects_invalid_yaml() {
        assert!(migrate_content("key: [unclosed").is_err());
        assert!(migrate_content("{broken").is_err());
        assert!(migrate_content("- just\n- a\n- list\n").is_err());
        assert!(migrate_content("").is_err());
    }

    /// 脱敏占位 secret 不应覆盖真实密钥（前端回传场景的防御）。
    /// 注意占位符以 `*` 开头，YAML 中必须加引号（否则是 alias 语法）。
    #[test]
    fn redacted_secret_not_merged() {
        let yaml = format!(
            "mixed-port: 7890\nsecret: \"{}\"\n",
            crate::config::model::SECRET_REDACTED
        );
        let config = migrate_content(&yaml).unwrap();
        assert_ne!(config.proxy.secret, crate::config::model::SECRET_REDACTED);
    }
}
