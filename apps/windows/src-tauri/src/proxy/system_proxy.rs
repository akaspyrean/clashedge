//! Windows system proxy via winreg (Internet Settings)
//!
//! 通过 Windows 注册表直接配置系统代理（HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings）。
//!
//! 语义（与 Clash 系客户端一致，参考 clash-verge / clash-nyanpasu）：
//! - 启用：写 ProxyServer = 127.0.0.1:<mixed-port>、ProxyOverride = 绕过列表、ProxyEnable = 1，
//!   并删除 AutoConfigURL（禁用用户原有 PAC，避免双重代理冲突）；
//! - 普通禁用：**仅置 ProxyEnable = 0**，不删除 ProxyServer / ProxyOverride；
//! - ownership 释放：调用方确认当前值仍由 ClashEdge 管理后，通过
//!   `restore_system_proxy` 精确写回 journal 中的完整快照。
//! - 不做 netsh winhttp：那需要管理员权限，且改的是机器级 WinHTTP 代理，
//!   与应用级系统代理无关（原实现的非致命调用容易失败并制造假象）。
//! - UWP 回环豁免在 proxy/loopback.rs，此处不重复实现。

use serde::{Deserialize, Serialize};

use crate::util::error::{Error, Result};

/// 系统代理配置结构（快照/还原用）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct SystemProxyConfig {
    /// 是否启用系统代理（ProxyEnable == 1）
    pub enabled: bool,
    /// 代理服务器地址（ProxyServer）
    pub address: String,
    /// 直连域名列表（ProxyOverride，标准分号分隔；读取兼容历史逗号）
    pub bypass_list: Vec<String>,
    /// PAC 脚本地址（AutoConfigURL；快照/还原语义下保留原值不覆盖）
    #[serde(default)]
    pub auto_config_url: Option<String>,
}

const KEY_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Internet Settings";

/// ClashEdge 接管期间写入 Windows 的完整、可比较状态。
///
/// ownership 判定必须比较完整状态，而不只是 `ProxyServer`。这样用户或其他软件
/// 在运行期间修改 ProxyOverride / PAC 后，退出路径也会认定 ownership 已丢失，
/// 不会覆盖对方的后续修改。
pub fn managed_proxy_config(mixed_port: u16) -> SystemProxyConfig {
    SystemProxyConfig {
        enabled: true,
        address: format!("127.0.0.1:{}", mixed_port),
        bypass_list: vec![
            "<local>".to_string(),
            "127.0.0.1".to_string(),
            "localhost".to_string(),
            "*.tauri.localhost".to_string(),
        ],
        auto_config_url: None,
    }
}

/// 没有可读旧快照时的安全“无代理”目标（仅用于兼容旧 journal）。
pub fn disabled_proxy_config() -> SystemProxyConfig {
    SystemProxyConfig {
        enabled: false,
        address: String::new(),
        bypass_list: Vec::new(),
        auto_config_url: None,
    }
}

/// Windows ProxyOverride 的标准分隔符。注册表写入统一使用分号，与
/// Windows Internet Settings 及主流 Clash 系客户端一致；逗号是历史遗留。
const BYPASS_SEPARATOR: &str = ";";

/// 把绕过列表序列化为 ProxyOverride 字符串（标准分号分隔）。
fn join_bypass_list(bypass_list: &[String]) -> String {
    bypass_list.join(BYPASS_SEPARATOR)
}

/// 解析 ProxyOverride 字符串为绕过列表。
///
/// 兼容三种历史形态：标准分号 `;`、遗留逗号 `,`、以及两者混合。
/// 空字符串返回空列表；各段做 trim 并丢弃空段（容错历史/外部写入的空白）。
fn parse_bypass_list(raw: &str) -> Vec<String> {
    if raw.is_empty() {
        return Vec::new();
    }
    raw.split([';', ','])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// 打开 Internet Settings 注册表键。
/// 必须带 KEY_WRITE：winreg 0.11 的 `open_subkey` 默认只以 KEY_READ 打开，
/// 之后 `set_value` 会因拒绝访问（ERROR_ACCESS_DENIED / error 5）失败，
/// 这正是"系统代理"开关报 error 5 的根因。
fn open_key() -> Result<winreg::RegKey> {
    open_key_at(KEY_PATH)
        .map_err(|e| Error::Other(format!("Failed to open registry key {}: {}", KEY_PATH, e)))
}

/// 打开指定注册表键（生产路径与测试子键共用）。
fn open_key_at(subkey: &str) -> Result<winreg::RegKey> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
    winreg::RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(subkey, KEY_READ | KEY_WRITE)
        .map_err(|e| Error::Other(format!("Failed to open registry key {}: {}", subkey, e)))
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

/// 精确恢复一份已经持久化的 Windows 代理快照。
///
/// 该函数会写回 ProxyServer、ProxyOverride、ProxyEnable 与 AutoConfigURL 的完整
/// 状态（快照无 PAC 时会删除 AutoConfigURL）。它只能由已经确认 ownership 的
/// journal 路径调用；普通“关闭”不能用它覆盖未知的用户状态。
pub fn restore_system_proxy(snapshot: &SystemProxyConfig) -> Result<()> {
    let key = open_key()?;
    restore_to_key(&key, snapshot)
}

/// 把代理设置写入指定的注册表键（生产 / 测试子键共用）。见 `set_system_proxy` 语义。
#[cfg(test)]
fn apply_to_key(
    key: &winreg::RegKey,
    enabled: bool,
    address: &str,
    bypass_list: &[String],
    auto_config_url: Option<&str>,
) -> Result<()> {
    if enabled {
        // 先删 AutoConfigURL：PAC 与静态代理并存时 WinINet 行为不确定，
        // 接管期间必须保证只有我们的静态代理生效。
        let _ = key.delete_value("AutoConfigURL");
        key.set_value("ProxyServer", &address)
            .map_err(|e| Error::Other(format!("Failed to set ProxyServer: {}", e)))?;
        let bypass_str = join_bypass_list(bypass_list);
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

fn restore_to_key(key: &winreg::RegKey, snapshot: &SystemProxyConfig) -> Result<()> {
    key.set_value("ProxyServer", &snapshot.address)
        .map_err(|e| Error::Other(format!("Failed to restore ProxyServer: {}", e)))?;
    key.set_value("ProxyOverride", &join_bypass_list(&snapshot.bypass_list))
        .map_err(|e| Error::Other(format!("Failed to restore ProxyOverride: {}", e)))?;
    match snapshot.auto_config_url.as_deref() {
        Some(url) => key
            .set_value("AutoConfigURL", &url)
            .map_err(|e| Error::Other(format!("Failed to restore AutoConfigURL: {}", e)))?,
        None => {
            let _ = key.delete_value("AutoConfigURL");
        }
    }
    key.set_value("ProxyEnable", &u32::from(snapshot.enabled))
        .map_err(|e| Error::Other(format!("Failed to restore ProxyEnable: {}", e)))?;

    #[cfg(target_os = "windows")]
    notify_wininet_changed();
    Ok(())
}

/// 获取当前系统代理状态（含 PAC 脚本地址，供启动快照 / 退出还原使用）
pub fn get_system_proxy() -> Result<SystemProxyConfig> {
    let key = open_key()?;
    read_from_key(&key)
}

/// 从指定的注册表键读取代理状态（生产 / 测试子键共用）。见 `get_system_proxy`。
fn read_from_key(key: &winreg::RegKey) -> Result<SystemProxyConfig> {
    let enabled: u32 = key.get_value("ProxyEnable").unwrap_or(0);
    let address: String = key.get_value("ProxyServer").unwrap_or_default();
    let bypass: String = key.get_value("ProxyOverride").unwrap_or_default();
    let auto_config_url: Option<String> = key.get_value("AutoConfigURL").ok();

    let bypass_list: Vec<String> = parse_bypass_list(&bypass);

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
        assert_eq!(join_bypass_list(&bypass), "<local>;lan");
    }

    // A：新写入的 ProxyOverride 必须统一使用标准分号分隔。
    #[test]
    fn join_bypass_list_uses_semicolon_separator() {
        let s = join_bypass_list(&[
            "<local>".to_string(),
            "127.0.0.1".to_string(),
            "localhost".to_string(),
            "*.tauri.localhost".to_string(),
        ]);
        assert_eq!(s, "<local>;127.0.0.1;localhost;*.tauri.localhost");
    }

    // A：managed 状态的 bypass 序列化后必须是规范的分号串。
    #[test]
    fn managed_proxy_config_bypass_is_canonical_semicolon_form() {
        let cfg = managed_proxy_config(7890);
        assert_eq!(
            join_bypass_list(&cfg.bypass_list),
            "<local>;127.0.0.1;localhost;*.tauri.localhost"
        );
    }

    // A：写→读往返后 bypass_list 与写入一致（注册表层在下面的冒烟测试覆盖）。
    #[test]
    fn join_then_parse_roundtrips_bypass_list() {
        let list = vec![
            "<local>".to_string(),
            "127.0.0.1".to_string(),
            "localhost".to_string(),
            "*.tauri.localhost".to_string(),
        ];
        assert_eq!(parse_bypass_list(&join_bypass_list(&list)), list);
    }

    // B：历史遗留的逗号格式必须仍可读取。
    #[test]
    fn parse_bypass_list_reads_legacy_comma_form() {
        assert_eq!(
            parse_bypass_list("<local>,127.0.0.1,localhost"),
            vec![
                "<local>".to_string(),
                "127.0.0.1".to_string(),
                "localhost".to_string()
            ]
        );
    }

    // C：分号与逗号混合的格式必须可读取。
    #[test]
    fn parse_bypass_list_reads_mixed_separators() {
        assert_eq!(
            parse_bypass_list("<local>;127.0.0.1,localhost"),
            vec![
                "<local>".to_string(),
                "127.0.0.1".to_string(),
                "localhost".to_string()
            ]
        );
        // 反向混合同样成立。
        assert_eq!(
            parse_bypass_list("<local>,127.0.0.1;localhost"),
            vec![
                "<local>".to_string(),
                "127.0.0.1".to_string(),
                "localhost".to_string()
            ]
        );
    }

    // C：标准分号格式可读取。
    #[test]
    fn parse_bypass_list_reads_semicolon_form() {
        assert_eq!(
            parse_bypass_list("<local>;127.0.0.1;localhost;*.tauri.localhost"),
            vec![
                "<local>".to_string(),
                "127.0.0.1".to_string(),
                "localhost".to_string(),
                "*.tauri.localhost".to_string()
            ]
        );
    }

    // D：空 ProxyOverride 正常处理为空列表。
    #[test]
    fn parse_bypass_list_handles_empty_string() {
        assert!(parse_bypass_list("").is_empty());
        // 仅分隔符/空白也应被规整为空列表。
        assert!(parse_bypass_list(";,").is_empty());
        assert!(parse_bypass_list(" ; ").is_empty());
    }

    #[test]
    fn managed_proxy_config_is_the_canonical_owned_state() {
        let managed = managed_proxy_config(7890);
        assert!(managed.enabled);
        assert_eq!(managed.address, "127.0.0.1:7890");
        assert_eq!(
            managed.bypass_list,
            vec!["<local>", "127.0.0.1", "localhost", "*.tauri.localhost"]
        );
        assert!(managed.auto_config_url.is_none());
    }

    /// Windows Registry 冒烟测试：对**独立的测试子键**做真实的写→读→删，
    /// 验证 `apply_to_key` / `read_from_key` 与 winreg 的真实往返，而不是只测
    /// 纯数据结构。绝不动用户真实的 Internet Settings 键（`KEY_PATH`）。
    ///
    /// 需要 HKCU 可写（正常用户上下文即可，无需管理员）。
    /// 通过检查：写入 enabled=true→读取 enabled/address/bypass 一致；
    /// 写入 enabled=false→ProxyEnable 清 0 且 PAC 写回。
    #[test]
    fn registry_apply_and_read_roundtrip_on_test_subkey() {
        use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};

        // 单层时间戳子键：delete_subkey_all 恰好删掉整棵测试键，不残留中间父层；
        // 并行/多次运行互不冲突。
        let subkey = format!(
            r"Software\ClashEdgeTestSysProxy-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let base = winreg::RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) = base
            .create_subkey_with_flags(&subkey, KEY_READ | KEY_WRITE)
            .expect("create test subkey under HKCU");

        let baseline = read_from_key(&key).expect("read baseline from empty test subkey");
        assert!(!baseline.enabled, "fresh subkey must not be enabled");
        assert!(baseline.auto_config_url.is_none());

        // 1. 写入 enabled=true：ProxyServer / ProxyOverride / ProxyEnable=1
        apply_to_key(
            &key,
            true,
            "127.0.0.1:7890",
            &["<local>".to_string(), "localhost".to_string()],
            None,
        )
        .expect("apply enabled");
        // A：写入的 ProxyOverride 原始值必须使用标准分号，而非历史逗号。
        let raw_override: String = key
            .get_value("ProxyOverride")
            .expect("read raw ProxyOverride");
        assert_eq!(raw_override, "<local>;localhost");
        let enabled = read_from_key(&key).expect("read enabled");
        assert!(enabled.enabled);
        assert_eq!(enabled.address, "127.0.0.1:7890");
        assert_eq!(
            enabled.bypass_list,
            vec!["<local>".to_string(), "localhost".to_string()]
        );

        // 精确快照恢复必须把全部字段（包括 disabled 状态下的静态值和 PAC）写回。
        let exact = SystemProxyConfig {
            enabled: false,
            address: "10.0.0.5:8080".to_string(),
            bypass_list: vec!["intranet.example".to_string()],
            auto_config_url: Some("http://pac.example/proxy.pac".to_string()),
        };
        restore_to_key(&key, &exact).expect("restore exact snapshot");
        assert_eq!(read_from_key(&key).unwrap(), exact);

        // 无 PAC 快照必须删除接管/外部操作残留的 AutoConfigURL。
        let no_pac = disabled_proxy_config();
        restore_to_key(&key, &no_pac).expect("restore snapshot without PAC");
        assert_eq!(read_from_key(&key).unwrap(), no_pac);

        // 2. 写入 enabled=false 且带原 PAC：ProxyEnable=0 + AutoConfigURL 写回
        apply_to_key(&key, false, "", &[], Some("http://127.0.0.1:1080/pac"))
            .expect("apply disabled with pac");
        let disabled = read_from_key(&key).expect("read disabled");
        assert!(!disabled.enabled);
        assert_eq!(
            disabled.auto_config_url.as_deref(),
            Some("http://127.0.0.1:1080/pac")
        );

        // 3. 写入 enabled=false 且无 PAC：ProxyEnable 清 0。AutoConfigURL 保持
        //    上一步写入的值不变——apply_to_key 的语义是「调用方无 PAC 快照时
        //    不碰注册表里可能存在的其他值」，因此第 2 步写入的 PAC 应保留。
        apply_to_key(&key, false, "", &[], None).expect("apply disabled no pac");
        let cleared = read_from_key(&key).expect("read cleared");
        assert!(!cleared.enabled);
        assert_eq!(
            cleared.auto_config_url.as_deref(),
            Some("http://127.0.0.1:1080/pac"),
            "disable without PAC snapshot must not clobber existing AutoConfigURL"
        );

        // 清理：先释放 key 句柄再删除子键——Windows 上不能删除仍被句柄占用的键，
        // winreg 的 delete_subkey_all 在句柄未 drop 时会静默失败导致测试子键泄漏。
        // 用 expect 断言删除成功，泄漏即测试失败。
        drop(key);
        base.delete_subkey_all(&subkey)
            .expect("test subkey must be removed to avoid leaking into HKCU");
    }
}
