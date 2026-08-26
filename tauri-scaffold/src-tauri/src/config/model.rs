// src-tauri/src/config/model.rs
//! 配置数据模型：对应 profile-preprocessor.cjs 处理后的配置结构
//! 单一来源：Rust 结构体 ←→ 前端 TypeScript 接口

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 完整配置结构（对应 config.yaml 顶层字段 + 应用 UI 设置）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Config {
    /// 常规设置（flatten 到顶层：mixed-port / allow-lan / mode ...）
    #[serde(flatten)]
    pub general: GeneralConfig,

    /// 外部控制器（mihomo 顶层键 external-controller / secret）
    /// 使用 flatten 与 config.yaml 顶层布局对齐：R8.3 把这些键放在顶层，
    /// 若嵌套在 proxy: 下，serde 会把未知顶层键静默丢弃，写回时值丢失且错位。
    #[serde(default, flatten)]
    pub proxy: ProxyConfig,

    /// TUN 模式
    #[serde(default)]
    pub tun: TunConfig,

    /// DNS
    #[serde(default)]
    pub dns: DnsConfig,

    /// 高级设置
    #[serde(default)]
    pub advanced: AdvancedConfig,

    /// 配置文件管理
    #[serde(default)]
    pub profiles: ProfilesConfig,

    /// 配置混入开关（应用级）
    #[serde(default)]
    pub mixin_enabled: bool,

    /// 界面语言
    #[serde(default = "default_locale")]
    pub locale: String,

    /// 规则提供者（订阅来源；内置基线 5 组 direct/ai/media/proxy/ad，
    /// 对应 profile-preprocessor.cjs 的 rule-providers 段）
    #[serde(default = "default_rule_providers")]
    pub rule_providers: HashMap<String, serde_yaml::Value>,

    /// 代理组（内置基线 6 组：GLOBAL + 扶梯出行/人工智能/影音视听/人工优选/自动优选；
    /// GLOBAL 为全局模式专用组，仅在配置中存在、不参与代理页面展示；
    /// 订阅导入后由前端/导入流程填充具体节点）
    #[serde(default = "default_proxy_groups")]
    pub proxy_groups: Vec<serde_yaml::Value>,

    /// 内置路由规则（对应 profile-preprocessor.cjs buildPreset 的内置段，
    /// 保证无订阅时也有完整分流规则而非空配置；订阅规则由导入流程前置插入）
    #[serde(default = "default_rules")]
    pub rules: Vec<String>,

    /// 兜底字段：订阅 / 导入的 mihomo 配置中，凡是结构体未建模的顶层键
    /// （如 `proxies`、`proxy-providers`、`hosts`、`sniffer` 等）都会进入此映射，
    /// 保存时原样写回，保证"导入 → 保存 → 重启"过程中未知字段不丢失。
    /// 这是 AppConfig 与 MihomoConfig 分离的字段保真基石。
    #[serde(default, flatten)]
    pub extra: serde_yaml::Mapping,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            general: GeneralConfig::default(),
            proxy: ProxyConfig::default(),
            tun: TunConfig::default(),
            dns: DnsConfig::default(),
            advanced: AdvancedConfig::default(),
            profiles: ProfilesConfig::default(),
            mixin_enabled: false,
            locale: default_locale(),
            rule_providers: default_rule_providers(),
            proxy_groups: default_proxy_groups(),
            rules: default_rules(),
            extra: serde_yaml::Mapping::new(),
        }
    }
}

pub fn default_locale() -> String {
    "zh-CN".to_string()
}

/// --- GeneralConfig ---
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct GeneralConfig {
    #[serde(default = "default_mixed_port")]
    pub mixed_port: u16,

    #[serde(default)]
    pub allow_lan: bool,

    /// P1-5：allow-lan 高级控制——绑定地址（mihomo `bind-address`）。
    /// None/空 → 不写该键（mihomo 默认绑定所有接口）；
    /// 常用值："*"（所有接口）、"127.0.0.1"（仅本机，等价关闭 LAN 访问）、
    /// 具体内网 IP。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bind_address: Option<String>,

    /// P1-5：允许访问代理的来源网段白名单（mihomo `lan-allowed-ips`，
    /// CIDR 列表如 192.168.1.0/24）。空列表 → 不写该键（mihomo 默认
    /// 0.0.0.0/0 + ::/0 即全部放行）。仅在 allow-lan=true 时写入运行时。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lan_allowed_ips: Vec<String>,

    #[serde(default = "default_log_level")]
    pub log_level: String,

    #[serde(default)]
    pub ipv6: bool,

    /// GeoData 更新来源模式（应用级：manual / use-external / remote）。
    /// 用 `serde_yaml::Value` 承载：订阅 / 导入的 mihomo 配置里 `geodata-mode`
    /// 常是 bool（true/false）或 "metax"/"v2ray" 字符串——若建模为 String，
    /// `geodata-mode: true` 会使整份配置反序列化失败、触发迁移并丢配置。
    /// Value 类型可同时保真应用级字符串与 mihomo 语义值；应用读取用 `.as_str()`。
    #[serde(default = "default_geodata_mode")]
    pub geodata_mode: serde_yaml::Value,

    #[serde(default)]
    pub geo_auto_update: bool,

    /// 启动时是否自动刷新过期订阅（超过 24h 未更新的订阅）。
    /// 隐式访问订阅服务器涉及可预期性与隐私，故做成显式设置；默认开启。
    #[serde(default = "default_auto_update_subscription")]
    pub auto_update_subscription: bool,

    #[serde(default = "default_find_process_mode")]
    pub find_process_mode: String,

    /// 代理模式。mihomo 顶层键为 `mode`（rule/global/direct），
    /// 旧版本（R8.3 及更早）写的是 `proxy-mode`，因此保留 `alias` 兼容旧配置。
    #[serde(default = "default_proxy_mode", rename = "mode", alias = "proxy-mode")]
    pub proxy_mode: String,

    #[serde(default)]
    pub profile: String,

    /// 系统代理开关（应用级状态，与 mihomo 的 allow-lan 分离）。
    /// 对应 Windows 注册表 Internet Settings 的 ProxyEnable；
    /// 不是 mihomo 顶层键，不会进入运行时配置。前端/托盘开关读此值，
    /// 实际生效由 `core::runtime::apply_system_proxy` 写注册表完成。
    #[serde(default)]
    pub system_proxy: bool,
}

fn default_mixed_port() -> u16 {
    7890
}
fn default_log_level() -> String {
    "info".to_string()
}
pub(crate) fn default_geodata_mode() -> serde_yaml::Value {
    serde_yaml::Value::String("manual".to_string())
}
fn default_find_process_mode() -> String {
    "off".to_string()
}
fn default_auto_update_subscription() -> bool {
    true
}
fn default_proxy_mode() -> String {
    "rule".to_string()
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            mixed_port: default_mixed_port(),
            allow_lan: false,
            bind_address: None,
            lan_allowed_ips: Vec::new(),
            log_level: default_log_level(),
            ipv6: false,
            geodata_mode: default_geodata_mode(),
            geo_auto_update: false,
            auto_update_subscription: true,
            find_process_mode: default_find_process_mode(),
            proxy_mode: default_proxy_mode(),
            profile: String::new(),
            system_proxy: false,
        }
    }
}

/// --- ProxyConfig ---
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ProxyConfig {
    /// 是否启用外部控制器
    #[serde(default = "default_external_controller")]
    pub external_controller: String,

    /// 外部控制器密钥
    #[serde(default = "default_secret")]
    pub secret: String,
}

/// 派生 Default 会把两个字段初始化为空串（绕过字段级 default=...），
/// 导致新配置写出的 external-controller 为空。手写 Default 对齐字段默认值。
impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            external_controller: default_external_controller(),
            secret: default_secret(),
        }
    }
}

fn default_external_controller() -> String {
    "127.0.0.1:9090".to_string()
}

/// 固定占位密钥：仅供 `Config::default()` 与旧配置迁移兜底，
/// 真实安装初始化时被 `ConfigManager::init` 轮换为随机值。
pub(crate) fn default_secret_placeholder() -> &'static str {
    "clash-edge-secret"
}

/// P0-3 脱敏占位符：`get_config` 返回给前端时用它替代真实 secret。
/// `update_config`/`import_config` 见到此值或空串时保留现有真实密钥，不轮换。
pub const SECRET_REDACTED: &str = "********";

fn default_secret() -> String {
    default_secret_placeholder().to_string()
}

/// 生成随机控制器密钥（32 位十六进制，128 位熵）。
/// `default_secret` 是占位值；真实安装首次初始化 / 旧版固定默认值会被
/// `ConfigManager::init` 轮换为随机值，避免控制器被已知默认密钥接管。
pub fn generate_random_secret() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// 是否需要轮换控制器密钥：空 / 固定占位 / 旧版（Clash.F.Win）遗留固定密钥。
/// `ConfigManager::ensure_secure_secret`（H1 收敛的落盘前统一轮换）
/// 与系统代理开启前兜底（C9）复用同一判定，避免两处判定逻辑漂移。
pub fn needs_secret_rotation(secret: &str) -> bool {
    secret.is_empty() || secret == default_secret_placeholder() || secret == "clash-f-win-secret"
}

/// 校验外部控制器地址：仅允许回环地址（`127.0.0.1:<port>` / `localhost:<port>` /
/// `[::1]:<port>`，port 1-65535）。
///
/// 导入 / 更新配置是用户可控输入，落盘前校验可防止把控制器指到局域网/公网地址
/// （可被第三方接管）或 `0.0.0.0`（任意接口监听）。
pub fn validate_external_controller(addr: &str) -> crate::util::error::Result<()> {
    use crate::util::error::Error;

    let trimmed = addr.trim();
    if trimmed.contains("://") {
        return Err(Error::InvalidArgument(format!(
            "external-controller must be host:port, got '{}'",
            addr
        )));
    }
    let (host, port_str) = if let Some(rest) = trimmed.strip_prefix('[') {
        // IPv6 字面量：[::1]:9090
        let Some((ipv6, port)) = rest.split_once("]:") else {
            return Err(Error::InvalidArgument(format!(
                "invalid external-controller: '{}'",
                addr
            )));
        };
        (ipv6, port)
    } else {
        let Some((h, p)) = trimmed.rsplit_once(':') else {
            return Err(Error::InvalidArgument(format!(
                "invalid external-controller (missing port): '{}'",
                addr
            )));
        };
        (h, p)
    };

    let port: u16 = port_str.parse().map_err(|_| {
        Error::InvalidArgument(format!("invalid port in external-controller: '{}'", addr))
    })?;
    if port == 0 {
        return Err(Error::InvalidArgument(format!(
            "external-controller port must be 1-65535: '{}'",
            addr
        )));
    }
    if !matches!(host, "127.0.0.1" | "localhost" | "::1") {
        return Err(Error::InvalidArgument(format!(
            "external-controller must be loopback (127.0.0.1 / localhost / [::1]): '{}'",
            addr
        )));
    }
    Ok(())
}

/// --- TunConfig ---
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct TunConfig {
    /// 是否启用 TUN 模式
    #[serde(default)]
    pub enable: bool,

    /// TUN 网卡类型：system 或 gvisor
    #[serde(default = "default_tun_stack")]
    pub stack: String,

    /// 自动路由
    #[serde(default)]
    pub auto_route: bool,

    /// 自动检测网卡
    #[serde(default)]
    pub auto_detect_interface: bool,

    /// 网卡名称（当 auto_detect_interface 为 false 时使用）
    #[serde(default)]
    pub interface_name: Option<String>,
}

/// 派生 Default 会让 stack 变成空串，写进 config.yaml 后 mihomo 报
/// "invalid tun stack" 拒绝启动。手写 Default 对齐字段默认值。
impl Default for TunConfig {
    fn default() -> Self {
        Self {
            enable: false,
            stack: default_tun_stack(),
            auto_route: false,
            auto_detect_interface: false,
            interface_name: None,
        }
    }
}

fn default_tun_stack() -> String {
    "system".to_string()
}

/// --- DnsConfig ---
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct DnsConfig {
    /// 是否启用 DNS
    #[serde(default = "default_dns_enable")]
    pub enable: bool,

    /// DNS 监听地址
    #[serde(default = "default_dns_listen")]
    pub listen: String,

    /// IPv6 启用
    #[serde(default)]
    pub ipv6: bool,

    /// 增强模式：fake-ip 或 fallback
    #[serde(default = "default_dns_enhanced_mode")]
    pub enhanced_mode: String,

    /// 假 IP 范围
    #[serde(default = "default_dns_fake_ip_range")]
    pub fake_ip_range: String,

    /// 假 IP 过滤器
    #[serde(default = "default_dns_fake_ip_filter")]
    pub fake_ip_filter: Vec<String>,

    /// 默认 nameserver
    #[serde(default = "default_dns_default_nameserver")]
    pub default_nameserver: Vec<String>,

    /// Nameserver（通过 DNS-over-HTTPS 等）
    #[serde(default = "default_dns_nameserver")]
    pub nameserver: Vec<String>,

    /// proxy-server-nameserver：解析代理节点服务器域名时使用的 DNS。
    /// 必须是明文 IP（非 DoH/DoT），mihomo 要求该字段仅接受明文 DNS。
    /// 避免节点域名解析走 DoH 导致循环依赖（DoH 域名自身需要先解析）。
    #[serde(default = "default_dns_proxy_server_nameserver")]
    pub proxy_server_nameserver: Vec<String>,
}

/// 派生 Default 会让 dns 所有字段变成空值，写进 config.yaml 后 mihomo 报
/// "NameServer cannot be empty" 等错误。手写 Default 对齐字段默认值。
impl Default for DnsConfig {
    fn default() -> Self {
        Self {
            enable: default_dns_enable(),
            listen: default_dns_listen(),
            ipv6: false,
            enhanced_mode: default_dns_enhanced_mode(),
            fake_ip_range: default_dns_fake_ip_range(),
            fake_ip_filter: default_dns_fake_ip_filter(),
            default_nameserver: default_dns_default_nameserver(),
            nameserver: default_dns_nameserver(),
            proxy_server_nameserver: default_dns_proxy_server_nameserver(),
        }
    }
}

fn default_dns_enable() -> bool {
    true
}
fn default_dns_listen() -> String {
    "127.0.0.1:9053".to_string()
}
fn default_dns_enhanced_mode() -> String {
    "fake-ip".to_string()
}
fn default_dns_fake_ip_range() -> String {
    "198.18.0.1/16".to_string()
}
fn default_dns_fake_ip_filter() -> Vec<String> {
    vec![
        "+.lan".to_string(),
        "+.local".to_string(),
        "+.home.arpa".to_string(),
        "localhost.ptlogin2.qq.com".to_string(),
        "+.msftconnecttest.com".to_string(),
        "+.msftncsi.com".to_string(),
        "*.n.n.srv.nintendo.net".to_string(),
    ]
}
fn default_dns_default_nameserver() -> Vec<String> {
    vec!["223.5.5.5".to_string(), "119.29.29.29".to_string()]
}
fn default_dns_nameserver() -> Vec<String> {
    vec![
        "https://dns.alidns.com/dns-query".to_string(),
        "https://doh.pub/dns-query".to_string(),
    ]
}
/// proxy-server-nameserver：解析代理节点服务器域名时使用的明文 DNS。
/// mihomo 要求该字段必须是明文 IP（非 DoH/DoT），否则会报错拒绝启动。
fn default_dns_proxy_server_nameserver() -> Vec<String> {
    vec!["223.5.5.5".to_string(), "119.29.29.29".to_string()]
}

/// --- AdvancedConfig ---
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct AdvancedConfig {
    /// 禁用提交按钮动画
    #[serde(default)]
    pub disable_commit_animation: bool,

    /// 日志输出格式
    #[serde(default = "default_log_format")]
    pub log_format: String,

    /// 是否显式代理
    #[serde(default)]
    pub explicit_proxy: bool,

    /// 连接超时（秒）
    #[serde(default = "default_advanced_connect_timeout")]
    pub connect_timeout: u64,

    /// 读取超时（秒）
    #[serde(default = "default_advanced_read_timeout")]
    pub read_timeout: u64,

    /// 写入超时（秒）
    #[serde(default = "default_advanced_write_timeout")]
    pub write_timeout: u64,

    /// Geox URL (GeoIP/GeoSite 下载地址)
    #[serde(default)]
    pub geox_url: String,

    /// GeoIP URL
    #[serde(default)]
    pub geoip_url: String,

    /// GeoSite URL
    #[serde(default)]
    pub geosite_url: String,
}

/// 派生 Default 会让超时字段变成 0、log_format 变空。手写 Default 对齐字段默认值。
impl Default for AdvancedConfig {
    fn default() -> Self {
        Self {
            disable_commit_animation: false,
            log_format: default_log_format(),
            explicit_proxy: false,
            connect_timeout: default_advanced_connect_timeout(),
            read_timeout: default_advanced_read_timeout(),
            write_timeout: default_advanced_write_timeout(),
            geox_url: String::new(),
            geoip_url: String::new(),
            geosite_url: String::new(),
        }
    }
}

fn default_log_format() -> String {
    "text".to_string()
}
fn default_advanced_connect_timeout() -> u64 {
    30
}
fn default_advanced_read_timeout() -> u64 {
    30
}
fn default_advanced_write_timeout() -> u64 {
    30
}

/// --- ProfilesConfig ---
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ProfilesConfig {
    /// 配置文件列表
    #[serde(default)]
    pub proxies: Vec<String>,

    /// 默认配置文件名
    #[serde(default = "default_profiles_default_profile")]
    pub default_profile: String,

    /// 自动模式组名称
    #[serde(default = "default_profiles_auto_group")]
    pub auto_group: String,

    /// 手动模式组名称
    #[serde(default = "default_profiles_manual_group")]
    pub manual_group: String,

    /// 自定义模式组名称
    #[serde(default = "default_profiles_media_group")]
    pub media_group: String,

    /// AI 模式组名称
    #[serde(default = "default_profiles_ai_group")]
    pub ai_group: String,
}

/// 派生 Default 会把 default-profile 等变成空串（字段级 default= 只在该节
/// 存在时生效）。手写 Default 对齐字段默认值。
impl Default for ProfilesConfig {
    fn default() -> Self {
        Self {
            proxies: Vec::new(),
            default_profile: default_profiles_default_profile(),
            auto_group: default_profiles_auto_group(),
            manual_group: default_profiles_manual_group(),
            media_group: default_profiles_media_group(),
            ai_group: default_profiles_ai_group(),
        }
    }
}

fn default_profiles_default_profile() -> String {
    "DIRECT".to_string()
}
fn default_profiles_auto_group() -> String {
    "自动".to_string()
}
fn default_profiles_manual_group() -> String {
    "手动".to_string()
}
fn default_profiles_media_group() -> String {
    "媒体".to_string()
}
fn default_profiles_ai_group() -> String {
    "AI".to_string()
}

/// 内置规则提供者：对应 profile-preprocessor.cjs buildPreset 的 rule-providers。
/// `path: ./rules/<name>.yaml` 相对 mihomo 的 `-d` 目录（Data/），随包附带这些文件，
/// 离线也能加载；联网后按 interval 自动更新。
pub fn default_rule_providers() -> HashMap<String, serde_yaml::Value> {
    const YAML: &str = r#"
direct:
  type: http
  behavior: classical
  url: https://raw.githubusercontent.com/akaspyrean/external/main/rules/direct.yaml
  path: ./rules/direct.yaml
  interval: 86400
ai:
  type: http
  behavior: classical
  url: https://raw.githubusercontent.com/akaspyrean/external/main/rules/ai.yaml
  path: ./rules/ai.yaml
  interval: 86400
media:
  type: http
  behavior: classical
  url: https://raw.githubusercontent.com/akaspyrean/external/main/rules/media.yaml
  path: ./rules/media.yaml
  interval: 86400
proxy:
  type: http
  behavior: classical
  url: https://raw.githubusercontent.com/akaspyrean/external/main/rules/proxy.yaml
  path: ./rules/proxy.yaml
  interval: 86400
ad:
  type: http
  behavior: classical
  url: https://raw.githubusercontent.com/akaspyrean/external/main/rules/ad.yaml
  path: ./rules/ad.yaml
  interval: 86400
"#;
    serde_yaml::from_str(YAML).unwrap_or_default()
}

/// 内置代理组：对应 profile-preprocessor.cjs 的 proxy-groups 段。
/// 叶子组初始为空（无订阅时无可用节点），订阅导入后由
/// `build_runtime_config` 注入真实节点名。
/// GLOBAL 为全局模式专用组：`mode: global` 时所有流量走它，只含 DIRECT/REJECT
/// 与两个叶子组；代理页面按模式联动（rule 隐藏 / global 独占显示 / direct 无组），
/// 由前端 `ProxiesView` 实现，托盘保留以便切换。
///
/// 注意：`自动优选` 是 `url-test` 类型，mihomo 仅对真实代理节点做延迟测速，
/// DIRECT 不是代理节点——注入 DIRECT 会让 url-test 把直连当作"零延迟最优节点"
/// 永久霸占自动组，所有真实节点永远拿不到流量。故订阅注入时只放真实节点名。
/// mihomo（v1.19.x）拒绝 proxies 为空的代理组，无订阅时的空列表由
/// `build_runtime_config` 在生成运行时配置时补 DIRECT 占位（仅零节点状态）。
pub fn default_proxy_groups() -> Vec<serde_yaml::Value> {
    const YAML: &str = r#"
- name: GLOBAL
  type: select
  proxies: [DIRECT, REJECT, 人工优选, 自动优选]
- name: 扶梯出行
  type: select
  proxies: [人工优选, 自动优选]
- name: 人工智能
  type: select
  proxies: [人工优选, 自动优选]
- name: 影音视听
  type: select
  proxies: [人工优选, 自动优选]
- name: 人工优选
  type: select
  proxies: []
- name: 自动优选
  type: url-test
  url: https://cp.cloudflare.com/generate_204
  interval: 300
  # tolerance=100ms：只有新节点比当前快 100ms 以上才切换，
  # 避免延迟抖动导致的频繁跳变；间隔 300s 兼顾反应速率与稳定性。
  # expected-status=204 + timeout=5000：与 cp.cloudflare.com 的
  # generate_204 端点契约对齐（仅 204 视为健康），并限制单次测速 5s，
  # 避免慢节点拖累整组切换决策（mihomo 当前版本支持这两个字段）。
  tolerance: 100
  expected-status: 204
  timeout: 5000
  proxies: []
"#;
    serde_yaml::from_str(YAML).unwrap_or_default()
}

/// 内置路由规则：对应 profile-preprocessor.cjs buildPreset 的内置规则段。
/// 采用经典分流：国内直连、广告拦截、AI/影音走专属组、其余（proxy 规则集 + MATCH）
/// 走"扶梯出行"主组。广告拦截双保险：内置 ad 规则集（external/ad.yaml，约 21 万条）
/// 优先于 GEOSITE category-ads-all 兜底，确保离线也有基础拦截。
pub fn default_rules() -> Vec<String> {
    vec![
        "GEOSITE,private,DIRECT".into(),
        "RULE-SET,direct,DIRECT".into(),
        "RULE-SET,ad,REJECT".into(),
        "GEOSITE,category-ads-all,REJECT".into(),
        "RULE-SET,ai,人工智能".into(),
        "RULE-SET,media,影音视听".into(),
        "RULE-SET,proxy,扶梯出行".into(),
        "GEOSITE,cn,DIRECT".into(),
        "GEOIP,CN,DIRECT".into(),
        "MATCH,扶梯出行".into(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// R8.3 config.yaml：external-controller / secret 位于顶层。
    /// 往返后必须保留原始值，且仍写回顶层（mihomo 可识别），
    /// 不得丢失、不得空串、不得错位嵌套到 proxy: 下。
    #[test]
    fn roundtrip_preserves_r83_external_controller() {
        let r83 = r#"
mixed-port: 7890
allow-lan: false
external-controller: 127.0.0.1:50715
secret: b1616fdd-63a8-44e9-b196-c63b68307a9b
log-level: error
"#;

        let config: Config = serde_yaml::from_str(r83).expect("parse R8.3 yaml");
        assert_eq!(config.proxy.external_controller, "127.0.0.1:50715");
        assert_eq!(config.proxy.secret, "b1616fdd-63a8-44e9-b196-c63b68307a9b");

        let yaml = serde_yaml::to_string(&config).expect("serialize");
        assert!(
            yaml.contains("external-controller: 127.0.0.1:50715"),
            "expected top-level external-controller in:\n{}",
            yaml
        );
        assert!(
            yaml.contains("secret: b1616fdd-63a8-44e9-b196-c63b68307a9b"),
            "expected top-level secret in:\n{}",
            yaml
        );
        // 不得出现嵌套 proxy 块：顶层一行恰好为 `proxy:`（注意 explicit-proxy 等
        // 字段名里也含 "proxy"，所以必须整行匹配而非子串匹配）
        assert!(
            !yaml.lines().any(|line| line == "proxy:"),
            "should not serialize a nested proxy block:\n{}",
            yaml
        );

        // 再解析一次，验证可循环（应用重载/重启路径）
        let again: Config = serde_yaml::from_str(&yaml).expect("re-parse round-tripped yaml");
        assert_eq!(again.proxy.external_controller, "127.0.0.1:50715");
        assert_eq!(again.proxy.secret, "b1616fdd-63a8-44e9-b196-c63b68307a9b");
    }

    /// 订阅/导入配置里的未知顶层键（proxies / proxy-providers / hosts 等）
    /// 必须经 `#[serde(flatten)] extra` 兜底，导入 → 保存 → 再解析 不丢失。
    #[test]
    fn unknown_top_level_keys_survive_roundtrip() {
        let yaml = r#"
mixed-port: 7890
mode: rule
proxies:
  - name: Node1
    type: ss
    server: 1.2.3.4
    port: 8388
    cipher: aes-128-gcm
    password: pwd
proxy-providers:
  myprovider:
    type: http
    url: https://example.com/x.yaml
    path: ./x.yaml
    interval: 86400
hosts:
  example.com: 1.2.3.4
"#;

        let config: Config = serde_yaml::from_str(yaml).expect("parse");
        assert!(
            config.extra.contains_key("proxies"),
            "proxies must be caught by extra"
        );
        assert!(
            config.extra.contains_key("proxy-providers"),
            "proxy-providers must be caught by extra"
        );
        assert!(
            config.extra.contains_key("hosts"),
            "hosts must be caught by extra"
        );
        assert_eq!(
            config.general.proxy_mode, "rule",
            "mode key maps to proxy_mode"
        );

        // 往返：保存后再解析，未知键与内容完整
        let out = serde_yaml::to_string(&config).expect("serialize");
        let again: Config = serde_yaml::from_str(&out).expect("re-parse");
        let proxies = again.extra.get("proxies").unwrap().as_sequence().unwrap();
        assert_eq!(proxies.len(), 1);
        assert_eq!(
            proxies[0].get("name").and_then(|n| n.as_str()),
            Some("Node1")
        );
        assert!(again.extra.contains_key("hosts"));
    }

    /// 旧配置键 `proxy-mode` 必须仍可被解析（alias 兼容 R8.3 及更早）。
    #[test]
    fn legacy_proxy_mode_alias_parses() {
        let yaml = "proxy-mode: global\nmixed-port: 7890\n";
        let config: Config = serde_yaml::from_str(yaml).expect("parse alias");
        assert_eq!(config.general.proxy_mode, "global");
        // 序列化应写出新键 `mode`，而非旧的 `proxy-mode`
        let out = serde_yaml::to_string(&config).expect("serialize");
        assert!(
            out.lines().any(|l| l == "mode: global"),
            "writes mode key:\n{}",
            out
        );
        assert!(
            !out.lines().any(|l| l == "proxy-mode: global"),
            "must not write legacy proxy-mode key:\n{}",
            out
        );
    }

    /// 订阅配置里的 `geodata-mode: true`（mihomo bool 语义）不得使整份解析失败，
    /// 且必须保真往返；应用级字符串值也应正常读取。
    #[test]
    fn geodata_mode_tolerates_mihomo_bool() {
        let yaml = "geodata-mode: true\nmixed-port: 7890\n";
        let config: Config = serde_yaml::from_str(yaml).expect("parse bool geodata-mode");
        assert_eq!(config.general.geodata_mode.as_bool(), Some(true));

        let out = serde_yaml::to_string(&config).expect("serialize");
        let again: Config = serde_yaml::from_str(&out).expect("re-parse");
        assert_eq!(again.general.geodata_mode.as_bool(), Some(true));

        // 应用级字符串值
        let yaml2 = "geodata-mode: manual\n";
        let config2: Config = serde_yaml::from_str(yaml2).expect("parse string geodata-mode");
        assert_eq!(config2.general.geodata_mode.as_str(), Some("manual"));
    }

    /// 全新默认配置的控制器默认值应为真实地址/密钥，而非空串。
    #[test]
    fn default_proxy_has_controller_defaults() {
        let config = Config::default();
        assert_eq!(config.proxy.external_controller, "127.0.0.1:9090");
        assert_eq!(config.proxy.secret, "clash-edge-secret");
    }

    /// 内置规则必须随默认配置写盘：rules / proxy-groups / rule-providers
    /// 都应有内容，且 MATCH 兜底存在（否则 mihomo 无规则、流量全直连）。
    #[test]
    fn default_config_has_builtin_rules() {
        let config = Config::default();

        assert!(!config.rules.is_empty(), "default rules should be built in");
        assert!(
            config.rules.iter().any(|r| r.starts_with("MATCH,")),
            "MATCH fallback rule must exist: {:?}",
            config.rules
        );
        assert_eq!(config.proxy_groups.len(), 6, "default proxy groups");
        assert_eq!(config.rule_providers.len(), 5, "default rule providers");
        // GLOBAL 组只含 DIRECT/REJECT/两个叶子组，不含节点
        let global = config
            .proxy_groups
            .iter()
            .find(|g| g.get("name").and_then(|n| n.as_str()) == Some("GLOBAL"))
            .expect("GLOBAL group must exist");
        let global_members: Vec<&str> = global
            .get("proxies")
            .and_then(|p| p.as_sequence())
            .map(|s| s.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();
        assert_eq!(
            global_members,
            vec!["DIRECT", "REJECT", "人工优选", "自动优选"],
            "GLOBAL group members: {:?}",
            global
        );

        // 序列化到 config.yaml 的键必须是 mihomo 顶层键
        let yaml = serde_yaml::to_string(&config).unwrap();
        for key in ["rules:", "proxy-groups:", "rule-providers:"] {
            assert!(
                yaml.lines().any(|l| l == key),
                "missing top-level {} in:\n{}",
                key,
                yaml
            );
        }
        // 内置分组名（扶梯出行等）不能被引号包裹——规则 RULE-SET 引用同名分组
        assert!(yaml.contains("扶梯出行"), "group name in yaml:\n{}", yaml);
        assert!(
            yaml.contains("RULE-SET,direct,DIRECT"),
            "builtin rule:\n{}",
            yaml
        );
        assert!(
            yaml.contains("RULE-SET,ad,REJECT"),
            "builtin ad rule:\n{}",
            yaml
        );
    }

    /// C7 合法回环控制器地址应全部通过。
    #[test]
    fn validate_external_controller_allows_loopback() {
        for addr in [
            "127.0.0.1:9090",
            "localhost:9090",
            "[::1]:9090",
            "127.0.0.1:1",
            "127.0.0.1:65535",
        ] {
            assert!(
                validate_external_controller(addr).is_ok(),
                "should allow loopback: {}",
                addr
            );
        }
    }

    /// C7 非回环 / 非法地址应被拒绝（http 前缀、0.0.0.0、私网、缺端口、端口 0）。
    #[test]
    fn validate_external_controller_rejects_non_loopback() {
        for addr in [
            "http://evil.com",
            "0.0.0.0:9090",
            "192.168.1.1:9090",
            "10.0.0.1:9090",
            "[::2]:9090",
            "127.0.0.1:0",
            "no-port",
        ] {
            assert!(
                validate_external_controller(addr).is_err(),
                "should reject non-loopback: {}",
                addr
            );
        }
    }
}
