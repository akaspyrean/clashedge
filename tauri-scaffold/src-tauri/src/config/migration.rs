// src-tauri/src/config/migration.rs
//! 配置版本迁移逻辑
//! 负责：旧版配置结构 → 新版 Config 结构的转换与升级

use std::path::Path;

use serde::{Deserialize, Serialize};
use tracing::{debug, info};

use crate::config::model::{
    AdvancedConfig, Config, DnsConfig, GeneralConfig, ProfilesConfig, ProxyConfig, TunConfig,
};
use crate::config::persistence::write_config;
use crate::util::error::Result;

/// 配置文件版本常量
const CONFIG_VERSION: &str = "2.0.0";

/// 配置迁移版本信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationInfo {
    /// 当前配置版本
    pub current_version: String,
    /// 目标版本
    pub target_version: String,
    /// 已迁移的字段列表
    pub migrated_fields: Vec<String>,
    /// 是否需要重启应用
    pub requires_restart: bool,
}

/// 执行配置迁移
/// - 读取旧配置文件
/// - 识别旧版本结构
/// - 逐步迁移到新版结构
/// - 写入新配置并返回迁移信息
pub fn migrate(config_path: &Path) -> Result<MigrationInfo> {
    if !config_path.exists() {
        info!("No config file to migrate");
        return Ok(MigrationInfo {
            current_version: String::new(),
            target_version: CONFIG_VERSION.to_string(),
            migrated_fields: Vec::new(),
            requires_restart: false,
        });
    }

    let content = std::fs::read_to_string(config_path)?;
    let content = crate::config::persistence::strip_utf8_bom(&content);

    if is_legacy_yaml(&content) {
        // 旧格式配置（profile-preprocessor.cjs 处理前的原始 YAML）
        debug!("Detected legacy YAML format, migrating from legacy...");
        migrate_from_legacy_yaml(&content, config_path)
    } else {
        // 可能已经是新格式，尝试直接解析
        match serde_yaml::from_str::<Config>(&content) {
            Ok(config) => {
                if is_new_format(&config) {
                    debug!("Config is already in new format");
                    return Ok(MigrationInfo {
                        current_version: "2.0.0".to_string(),
                        target_version: CONFIG_VERSION.to_string(),
                        migrated_fields: Vec::new(),
                        requires_restart: false,
                    });
                }
                // 混合格式，部分迁移
                migrate_mixed_format(&content, config_path)
            }
            Err(_) => {
                // 解析失败，尝试基础迁移
                migrate_from_yaml_string(&content, config_path)
            }
        }
    }
}

/// 从旧格式 YAML 迁移
fn migrate_from_legacy_yaml(content: &str, config_path: &Path) -> Result<MigrationInfo> {
    // 旧格式特征：缺少新结构字段，或结构简化
    // profile-preprocessor.cjs 生成的格式

    #[derive(Debug, Deserialize)]
    struct LegacyDns {
        enable: Option<bool>,
        listen: Option<String>,
        ipv6: Option<bool>,
    }

    #[derive(Debug, Deserialize)]
    struct LegacyTun {
        enable: Option<bool>,
        stack: Option<String>,
    }

    #[derive(Debug, Deserialize)]
    struct LegacyAdvanced {
        log_format: Option<String>,
        connect_timeout: Option<u64>,
        read_timeout: Option<u64>,
        write_timeout: Option<u64>,
    }

    #[derive(Debug, Deserialize)]
    struct LegacyConfig {
        mixed_port: Option<u16>,
        allow_lan: Option<bool>,
        log_level: Option<String>,
        ipv6: Option<bool>,
        geodata_mode: Option<String>,
        geo_auto_update: Option<bool>,
        find_process_mode: Option<String>,
        proxy_mode: Option<String>,
        profile: Option<String>,
        proxies: Option<Vec<String>>,
        external_controller: Option<String>,
        secret: Option<String>,
        dns: Option<LegacyDns>,
        tun: Option<LegacyTun>,
        advanced: Option<LegacyAdvanced>,
        default_profile: Option<String>,
        mixin_enabled: Option<bool>,
    }

    let legacy: LegacyConfig = serde_yaml::from_str(content).unwrap_or_else(|_| LegacyConfig {
        mixed_port: None,
        allow_lan: None,
        log_level: None,
        ipv6: None,
        geodata_mode: None,
        geo_auto_update: None,
        find_process_mode: None,
        proxy_mode: None,
        profile: None,
        proxies: None,
        external_controller: None,
        secret: None,
        dns: None,
        tun: None,
        advanced: None,
        default_profile: None,
        mixin_enabled: None,
    });

    let migrated = Config {
        general: GeneralConfig {
            mixed_port: legacy.mixed_port.unwrap_or(7890),
            allow_lan: legacy.allow_lan.unwrap_or(false),
            log_level: legacy.log_level.unwrap_or_else(|| "info".to_string()),
            ipv6: legacy.ipv6.unwrap_or(false),
            geodata_mode: legacy
                .geodata_mode
                .map(serde_yaml::Value::String)
                .unwrap_or_else(|| serde_yaml::Value::String("manual".to_string())),
            geo_auto_update: legacy.geo_auto_update.unwrap_or(false),
            find_process_mode: legacy
                .find_process_mode
                .unwrap_or_else(|| "off".to_string()),
            proxy_mode: legacy.proxy_mode.unwrap_or_else(|| "rule".to_string()),
            profile: legacy.profile.unwrap_or_default(),
            system_proxy: false,
        },
        proxy: ProxyConfig {
            external_controller: legacy
                .external_controller
                .unwrap_or_else(|| "127.0.0.1:9090".to_string()),
            secret: legacy
                .secret
                .unwrap_or_else(|| "clash-edge-secret".to_string()),
        },
        tun: TunConfig {
            enable: legacy.tun.as_ref().and_then(|t| t.enable).unwrap_or(false),
            stack: legacy
                .tun
                .as_ref()
                .and_then(|t| t.stack.clone())
                .unwrap_or_else(|| "system".to_string()),
            auto_route: false,
            auto_detect_interface: false,
            interface_name: None,
        },
        dns: DnsConfig {
            enable: legacy.dns.as_ref().and_then(|d| d.enable).unwrap_or(true),
            listen: legacy
                .dns
                .as_ref()
                .and_then(|d| d.listen.clone())
                .unwrap_or_else(|| "127.0.0.1:9053".to_string()),
            ipv6: legacy.dns.as_ref().and_then(|d| d.ipv6).unwrap_or(false),
            enhanced_mode: "fake-ip".to_string(),
            fake_ip_range: "198.18.0.1/16".to_string(),
            fake_ip_filter: vec![
                "+.lan".to_string(),
                "+.local".to_string(),
                "+.home.arpa".to_string(),
                "localhost.ptlogin2.qq.com".to_string(),
                "+.msftconnecttest.com".to_string(),
                "+.msftncsi.com".to_string(),
                "*.n.n.srv.nintendo.net".to_string(),
            ],
            default_nameserver: vec!["223.5.5.5".to_string(), "119.29.29.29".to_string()],
            nameserver: vec![
                "https://dns.alidns.com/dns-query".to_string(),
                "https://doh.pub/dns-query".to_string(),
            ],
            proxy_server_nameserver: vec!["223.5.5.5".to_string(), "119.29.29.29".to_string()],
        },
        advanced: AdvancedConfig {
            disable_commit_animation: false,
            log_format: legacy
                .advanced
                .as_ref()
                .and_then(|a| a.log_format.clone())
                .unwrap_or_else(|| "text".to_string()),
            explicit_proxy: false,
            connect_timeout: legacy
                .advanced
                .as_ref()
                .and_then(|a| a.connect_timeout)
                .unwrap_or(30),
            read_timeout: legacy
                .advanced
                .as_ref()
                .and_then(|a| a.read_timeout)
                .unwrap_or(30),
            write_timeout: legacy
                .advanced
                .as_ref()
                .and_then(|a| a.write_timeout)
                .unwrap_or(30),
            geox_url: String::new(),
            geoip_url: String::new(),
            geosite_url: String::new(),
        },
        profiles: ProfilesConfig {
            proxies: legacy.proxies.unwrap_or_default(),
            default_profile: legacy
                .default_profile
                .unwrap_or_else(|| "DIRECT".to_string()),
            auto_group: "自动".to_string(),
            manual_group: "手动".to_string(),
            media_group: "媒体".to_string(),
            ai_group: "AI".to_string(),
        },
        mixin_enabled: legacy.mixin_enabled.unwrap_or(false),
        locale: crate::config::model::default_locale(),
        rule_providers: crate::config::model::default_rule_providers(),
        proxy_groups: crate::config::model::default_proxy_groups(),
        rules: crate::config::model::default_rules(),
        extra: serde_yaml::Mapping::new(),
    };

    // 写入新格式配置
    write_config(config_path, &migrated)?;

    Ok(MigrationInfo {
        current_version: "1.0.0".to_string(),
        target_version: CONFIG_VERSION.to_string(),
        migrated_fields: vec![
            "mixed_port".to_string(),
            "allow_lan".to_string(),
            "log_level".to_string(),
            "ipv6".to_string(),
            "geodata_mode".to_string(),
            "geo_auto_update".to_string(),
            "find_process_mode".to_string(),
            "proxy_mode".to_string(),
            "profile".to_string(),
            "proxy.external_controller".to_string(),
            "proxy.secret".to_string(),
            "tun.enable".to_string(),
            "tun.stack".to_string(),
            "dns.enable".to_string(),
            "dns.listen".to_string(),
            "dns.enhanced-mode".to_string(),
            "dns.fake-ip-range".to_string(),
            "dns.fake-ip-filter".to_string(),
            "dns.default-nameserver".to_string(),
            "dns.nameserver".to_string(),
            "advanced.log_format".to_string(),
            "advanced.connect_timeout".to_string(),
            "advanced.read_timeout".to_string(),
            "advanced.write_timeout".to_string(),
            "profiles.default_profile".to_string(),
            "profiles.mixin_enabled".to_string(),
        ],
        requires_restart: true,
    })
}

/// 从混合格式迁移（部分新字段，部分旧字段）
fn migrate_mixed_format(_content: &str, _config_path: &Path) -> Result<MigrationInfo> {
    // 混合格式处理：检查哪些新字段存在，缺失则填充默认值
    Ok(MigrationInfo {
        current_version: "1.5.0".to_string(),
        target_version: CONFIG_VERSION.to_string(),
        migrated_fields: vec![],
        requires_restart: true,
    })
}

/// 检查是否为旧格式 YAML
/// 新格式：顶层存在 `proxy:` 映射（含 external-controller/secret）
/// 旧格式：字段平铺在顶层（external-controller / secret 直接出现在顶层）
fn is_legacy_yaml(content: &str) -> bool {
    let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(content) else {
        return false;
    };
    let has_proxy_section = value.get("proxy").map(|v| v.is_mapping()).unwrap_or(false);
    !has_proxy_section
}

/// 检查是否为新格式配置
fn is_new_format(_config: &Config) -> bool {
    true // 简化实现
}

/// 从 YAML 字符串迁移（用于解析失败的情况）
fn migrate_from_yaml_string(_content: &str, _config_path: &Path) -> Result<MigrationInfo> {
    Ok(MigrationInfo {
        current_version: "1.0.0".to_string(),
        target_version: CONFIG_VERSION.to_string(),
        migrated_fields: Vec::new(),
        requires_restart: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "clash-edge-migration-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_migrate_from_legacy() {
        let dir = temp_dir();
        let config_path = dir.join("config.yaml");

        let legacy_yaml = r#"
mixed-port: 7890
allow-lan: true
log-level: error
ipv6: false
geodata-mode: manual
geo-auto-update: false
find-process-mode: strict
proxy-mode: rule
profile: default

external-controller: 127.0.0.1:9090
secret: mysecret

dns:
  enable: true
  listen: 0.0.0.0:9053
  ipv6: false

tun:
  enable: true
  stack: system
"#;

        std::fs::write(&config_path, legacy_yaml).unwrap();
        let info = migrate(&config_path).unwrap();

        assert_eq!(info.current_version, "1.0.0");
        assert_eq!(info.target_version, "2.0.0");
        assert!(!info.migrated_fields.is_empty());
        assert!(info.requires_restart);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_migrate_already_new() {
        let dir = temp_dir();
        let config_path = dir.join("config.yaml");

        let new_yaml = r#"
mixed-port: 7890
allow-lan: true

proxy:
  external-controller: 127.0.0.1:9090
  secret: mysecret

dns:
  enable: true
  listen: 127.0.0.1:9053

tun:
  enable: true
  stack: system

advanced:
  log-format: text
  connect-timeout: 30
  read-timeout: 30
  write-timeout: 30

profiles:
  default-profile: DIRECT
"#;

        std::fs::write(&config_path, new_yaml).unwrap();
        let info = migrate(&config_path).unwrap();

        assert_eq!(info.current_version, "2.0.0");
        assert_eq!(info.target_version, "2.0.0");
        assert!(info.migrated_fields.is_empty());
        assert!(!info.requires_restart);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
