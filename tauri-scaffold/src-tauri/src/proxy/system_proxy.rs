//! Windows system proxy via winreg (Internet Settings)
//!
//! 通过 Windows 注册表直接配置系统代理（HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings）。
//!
//! 语义（与 Clash 系客户端一致，参考 clash-verge / clash-nyanpasu）：
//! - 启用：写 ProxyServer = 127.0.0.1:<mixed-port>、ProxyOverride = 绕过列表、ProxyEnable = 1，
//!   并删除 AutoConfigURL（禁用用户原有 PAC，避免双重代理冲突）；
//! - 禁用：**仅置 ProxyEnable = 0**，不删除 ProxyServer / ProxyOverride ——
//!   用户若原有自己的代理值，不会被我们清掉；快照中的原 AutoConfigURL 由调用方
//!   传入写回还原；退出还原按启动快照处理（见 main.rs）。
//! - 不做 netsh winhttp：那需要管理员权限，且改的是机器级 WinHTTP 代理，
//!   与应用级系统代理无关（原实现的非致命调用容易失败并制造假象）。
//! - UWP 回环豁免在 proxy/loopback.rs，此处不重复实现。

use serde::{Deserialize, Serialize};

use crate::util::error::{Error, Result};

/// 系统代理配置结构（快照/还原用）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct SystemProxyConfig {
    /// 是否启用系统代理（ProxyEnable == 1）
    pub enabled: bool,
    /// 代理服务器地址（ProxyServer）
    pub address: String,
    /// 直连域名列表（ProxyOverride，逗号分隔）
    pub bypass_list: Vec<String>,
    /// PAC 脚本地址（AutoConfigURL；快照/还原语义下保留原值不覆盖）
    #[serde(default)]
    pub auto_config_url: Option<String>,
}

const KEY_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Internet Settings";

/// 打开 Internet Settings 注册表键。
/// 必须带 KEY_WRITE：winreg 0.11 的 `open_subkey` 默认只以 KEY_READ 打开，
/// 之后 `set_value` 会因拒绝访问（ERROR_ACCESS_DENIED / error 5）失败，
/// 这正是"系统代理"开关报 error 5 的根因。
fn open_key() -> Result<winreg::RegKey> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
    winreg::RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(KEY_PATH, KEY_READ | KEY_WRITE)
        .map_err(|e| Error::Other(format!("Failed to open registry key {}: {}", KEY_PATH, e)))
}

/// 通知 WinINet 系统代理配置已变更（注册表写入后立即生效）。
/// 只改注册表时，已缓存代理设置的进程不会刷新；这两次全局 InternetSetOption
/// （hInternet = NULL：设置已变更 + 立即刷新缓存）与 clash-verge 等一致。
#[cfg(target_os = "windows")]
fn notify_wininet_changed() {
    use windows_sys::Win32::Networking::WinInet::{
        InternetSetOptionW, INTERNET_OPTION_REFRESH, INTERNET_OPTION_SETTINGS_CHANGED,
    };
    unsafe {
        let _ = InternetSetOptionW(
            std::ptr::null::<core::ffi::c_void>(),
            INTERNET_OPTION_SETTINGS_CHANGED,
            std::ptr::null::<core::ffi::c_void>(),
            0,
        );
        let _ = InternetSetOptionW(
            std::ptr::null::<core::ffi::c_void>(),
            INTERNET_OPTION_REFRESH,
            std::ptr::null::<core::ffi::c_void>(),
            0,
        );
    }
}

/// 启用或禁用系统代理。
///
/// - `enabled == true`：写 ProxyServer / ProxyOverride / ProxyEnable=1，并删除
///   AutoConfigURL（接管期间禁用用户原有 PAC，避免静态代理与 PAC 双重代理冲突；
///   原值已随启动快照 / journal 保留）；
/// - `enabled == false`：仅置 ProxyEnable=0，保留用户原有 ProxyServer / ProxyOverride；
///   若调用方持有快照中的原 AutoConfigURL（`auto_config_url`），写回以还原用户
///   原有 PAC。退出时的完整还原见 main.rs 快照。
pub fn set_system_proxy(
    enabled: bool,
    address: &str,
    bypass_list: &[String],
    auto_config_url: Option<&str>,
) -> Result<()> {
    let key = open_key()?;

    if enabled {
        // 先删 AutoConfigURL：PAC 与静态代理并存时 WinINet 行为不确定，
        // 接管期间必须保证只有我们的静态代理生效。
        let _ = key.delete_value("AutoConfigURL");
        key.set_value("ProxyServer", &address)
            .map_err(|e| Error::Other(format!("Failed to set ProxyServer: {}", e)))?;
        let bypass_str = bypass_list.join(",");
        key.set_value("ProxyOverride", &bypass_str)
            .map_err(|e| Error::Other(format!("Failed to set ProxyOverride: {}", e)))?;
        key.set_value("ProxyEnable", &1u32)
            .map_err(|e| Error::Other(format!("Failed to enable ProxyEnable: {}", e)))?;
    } else {
        key.set_value("ProxyEnable", &0u32)
            .map_err(|e| Error::Other(format!("Failed to clear ProxyEnable: {}", e)))?;
        // 快照里有原 PAC → 写回还原；没有则不动注册表里可能存在的其他值。
        if let Some(url) = auto_config_url {
            key.set_value("AutoConfigURL", &url)
                .map_err(|e| Error::Other(format!("Failed to restore AutoConfigURL: {}", e)))?;
        }
    }

    // 注册表写入成功后通知 WinINet，让系统代理立即生效（而非等缓存过期）。
    #[cfg(target_os = "windows")]
    notify_wininet_changed();

    Ok(())
}

/// 获取当前系统代理状态（含 PAC 脚本地址，供启动快照 / 退出还原使用）
pub fn get_system_proxy() -> Result<SystemProxyConfig> {
    let key = open_key()?;

    let enabled: u32 = key.get_value("ProxyEnable").unwrap_or(0);
    let address: String = key.get_value("ProxyServer").unwrap_or_default();
    let bypass: String = key.get_value("ProxyOverride").unwrap_or_default();
    let auto_config_url: Option<String> = key.get_value("AutoConfigURL").ok();

    let bypass_list: Vec<String> = if bypass.is_empty() {
        vec![]
    } else {
        bypass
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    };

    Ok(SystemProxyConfig {
        enabled: enabled == 1,
        address,
        bypass_list,
        auto_config_url,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_system_proxy_config_roundtrip() {
        let config = SystemProxyConfig {
            enabled: true,
            address: "127.0.0.1:7890".to_string(),
            bypass_list: vec!["<local>".to_string(), "192.168.0.0/16".to_string()],
            auto_config_url: Some("http://127.0.0.1:1080/pac".to_string()),
        };

        let serialized = serde_json::to_string(&config).unwrap();
        let deserialized: SystemProxyConfig = serde_json::from_str(&serialized).unwrap();

        assert!(deserialized.enabled);
        assert_eq!(deserialized.address, "127.0.0.1:7890");
        assert!(deserialized.bypass_list.contains(&"<local>".to_string()));
        assert_eq!(
            deserialized.auto_config_url.as_deref(),
            Some("http://127.0.0.1:1080/pac")
        );
    }

    #[test]
    fn test_legacy_journal_json_with_enable_loopback_still_deserializes() {
        // 旧版本 journal 里存有已移除的 enable_loopback 字段：
        // serde 默认忽略未知字段，旧 JSON 必须继续可反序列化。
        let legacy = r#"{
            "enabled": false,
            "address": "",
            "bypass-list": [],
            "auto-config-url": null,
            "enable_loopback": true
        }"#;
        let config: SystemProxyConfig = serde_json::from_str(legacy).unwrap();
        assert!(!config.enabled);
        assert!(config.auto_config_url.is_none());
    }

    #[test]
    fn test_missing_auto_config_url_defaults_to_none() {
        // 旧快照没有 auto-config-url 键 → #[serde(default)] 容错为 None
        let legacy = r#"{ "enabled": true, "address": "1.2.3.4:8080", "bypass-list": ["a"] }"#;
        let config: SystemProxyConfig = serde_json::from_str(legacy).unwrap();
        assert!(config.auto_config_url.is_none());
    }

    #[test]
    fn test_override_bypass_join() {
        let bypass = ["<local>".to_string(), "lan".to_string()];
        assert_eq!(bypass.join(","), "<local>,lan");
    }
}
