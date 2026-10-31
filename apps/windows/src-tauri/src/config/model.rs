// src-tauri/src/config/model.rs
//! 配置数据模型：config.yaml（应用持久化配置）的结构
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
    /// 使用 flatten 与 config.yaml 顶层布局对齐（这些键位于顶层）：
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

    /// 界面语言
    #[serde(default = "default_locale")]
    pub locale: String,

    /// 规则提供者（订阅来源；默认内置基线 5 组 direct/ai/media/proxy/ad，
    /// 保证无订阅时也有完整分流所需的 provider 定义，随包附带规则文件可离线加载）
    #[serde(default = "default_rule_providers")]
    pub rule_providers: HashMap<String, serde_norway::Value>,

    /// 代理组（内置基线 6 组：GLOBAL + 扶梯出行/人工智能/影音视听/人工优选/自动优选；
    /// GLOBAL 为全局模式专用组，仅在配置中存在、不参与代理页面展示；
    /// 订阅导入后由前端/导入流程填充具体节点）
    #[serde(default = "default_proxy_groups")]
    pub proxy_groups: Vec<serde_norway::Value>,

    /// 内置路由规则（内置基线，
    /// 保证无订阅时也有完整分流规则而非空配置；订阅规则由导入流程前置插入）
    #[serde(default = "default_rules")]
    pub rules: Vec<String>,

    /// 兜底字段：订阅 / 导入的 mihomo 配置中，凡是结构体未建模的顶层键
    /// （如 `proxies`、`proxy-providers`、`hosts`、`sniffer` 等）都会进入此映射，
    /// 保存时原样写回，保证"导入 → 保存 → 重启"过程中未知字段不丢失。
    /// 这是 AppConfig 与 MihomoConfig 分离的字段保真基石。
    #[serde(default, flatten)]
    pub extra: serde_norway::Mapping,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            general: GeneralConfig::default(),
            proxy: ProxyConfig::default(),
            tun: TunConfig::default(),
            dns: DnsConfig::default(),
            advanced: AdvancedConfig::default(),
            locale: default_locale(),
            rule_providers: default_rule_providers(),
            proxy_groups: default_proxy_groups(),
            rules: default_rules(),
            extra: serde_norway::Mapping::new(),
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

    /// allow-lan 高级控制——绑定地址（mihomo `bind-address`）。
    /// None/空 → 不写该键（mihomo 默认绑定所有接口）；
    /// 常用值："*"（所有接口）、"127.0.0.1"（仅本机，等价关闭 LAN 访问）、
    /// 具体内网 IP。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bind_address: Option<String>,

    /// 允许访问代理的来源网段白名单（mihomo `lan-allowed-ips`，
    /// CIDR 列表如 192.168.1.0/24）。空列表 → 不写该键（mihomo 默认
    /// 0.0.0.0/0 + ::/0 即全部放行）。仅在 allow-lan=true 时写入运行时。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lan_allowed_ips: Vec<String>,

    #[serde(default = "default_log_level")]
    pub log_level: String,

    #[serde(default)]
    pub ipv6: bool,

    /// GeoData 更新来源模式（应用级：manual / use-external / remote）。
    /// 用 `serde_norway::Value` 承载：订阅 / 导入的 mihomo 配置里 `geodata-mode`
    /// 常是 bool（true/false）或 "metax"/"v2ray" 字符串——若建模为 String，
    /// `geodata-mode: true` 会使整份配置反序列化失败、触发迁移并丢配置。
    /// Value 类型可同时保真应用级字符串与 mihomo 语义值；应用读取用 `.as_str()`。
    #[serde(default = "default_geodata_mode")]
    pub geodata_mode: serde_norway::Value,

    /// GeoData / 签名规则清单自动更新（启动后 90s 单次检查，>24h 才执行；
    /// 更新流先拉签名规则清单再补 GeoData）。默认开启。
    #[serde(default = "default_geo_auto_update")]
    pub geo_auto_update: bool,

    /// 启动时是否自动刷新过期订阅（超过 24h 未更新的订阅）。
    /// 隐式访问订阅服务器涉及可预期性与隐私，故做成显式设置；默认开启。
    #[serde(default = "default_auto_update_subscription")]
    pub auto_update_subscription: bool,

    /// 启动后静默检查一次更新（只检查并提示，不自动下载 / 安装）。
    /// 与订阅自动刷新一样是显式开关：无常驻定时器，仅启动后单次。
    #[serde(default = "default_auto_check_update")]
    pub auto_check_update: bool,

    /// 启用域名嗅探（TLS / HTTP / QUIC）：TUN 下纯 IP 连接也能按域名规则分流。
    /// 默认关闭以保持既有行为。订阅自带的 sniffer 配置仍一律不透传。
    #[serde(default)]
    pub sniffer: bool,

    #[serde(default = "default_find_process_mode")]
    pub find_process_mode: String,

    /// 代理模式。mihomo 顶层键为 `mode`（rule/global/direct），
    /// 旧配置写的 `proxy-mode` 键通过 `alias` 保持兼容。
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
    "error".to_string()
}
pub(crate) fn default_geodata_mode() -> serde_norway::Value {
    serde_norway::Value::String("manual".to_string())
}
fn default_find_process_mode() -> String {
    "off".to_string()
}
fn default_auto_update_subscription() -> bool {
    true
}
fn default_geo_auto_update() -> bool {
    true
}
fn default_auto_check_update() -> bool {
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
            geo_auto_update: default_geo_auto_update(),
            auto_update_subscription: true,
            auto_check_update: default_auto_check_update(),
            sniffer: false,
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

/// 脱敏占位符：`get_config` 返回给前端时用它替代真实 secret。
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
/// `ConfigManager::ensure_secure_secret`（落盘前统一轮换）
/// 与系统代理开启前兜底复用同一判定，避免两处判定逻辑漂移。
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
///
/// 面向普通 Windows 用户的默认值：TUN 默认关闭（enable=false），
/// 用户开启后 stack=mixed（优先同机 Wintun/原生驱动，复杂场景回退用户态栈）、
/// auto-route / auto-detect-interface 默认开启（开箱即用建立自动路由），
/// 并内置 DNS 劫持（any:53 + tcp://any:53）保证 TUN 内核接管系统 DNS。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct TunConfig {
    /// 是否启用 TUN 模式
    #[serde(default)]
    pub enable: bool,

    /// TUN 网卡类型：mihomo 只接受 mixed / system / gvisor。
    /// 非法值由 `merge_rules`（core/config.rs）归一为 `mixed`。
    #[serde(default = "default_tun_stack")]
    pub stack: String,

    /// 自动路由（开设/取消 Windows 默认路由表条目，交给 TUN 内核接管流量）
    #[serde(default = "default_tun_auto_route")]
    pub auto_route: bool,

    /// 自动检测网卡（auto-detect-interface；对普通用户免去手动指定网卡）
    #[serde(default = "default_tun_auto_detect_interface")]
    pub auto_detect_interface: bool,

    /// 网卡名称（当 auto_detect_interface 为 false 时使用）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interface_name: Option<String>,

    /// 严格路由（mihomo `strict-route`）：强制所有流量经 TUN，避免 Windows 多网卡
    /// 并行 DNS 解析造成的 DNS 泄漏；代价是可能影响个别依赖物理网卡直连的软件。
    /// 默认 false 以保持既有行为。
    #[serde(default = "default_tun_strict_route")]
    pub strict_route: bool,

    /// TUN 内核接管 DNS 的劫持地址列表（mihomo `dns-hijack`）。
    /// 默认 any:53 + tcp://any:53，接管本机所有 DNS 请求交给 mihomo DNS 处理，
    /// 否则 TUN 模式下请求会绕过代理直连或产生 DNS 泄漏。
    /// 正常用户无需编辑；老配置缺该字段时由 default 兜底。
    #[serde(default = "default_dns_hijack")]
    pub dns_hijack: Vec<String>,
}

/// 派生 Default 会让 stack 变成空串，写进 config.yaml 后 mihomo 报
/// "invalid tun stack" 拒绝启动。手写 Default 对齐字段默认值。
impl Default for TunConfig {
    fn default() -> Self {
        Self {
            enable: false,
            stack: default_tun_stack(),
            auto_route: default_tun_auto_route(),
            auto_detect_interface: default_tun_auto_detect_interface(),
            interface_name: None,
            strict_route: default_tun_strict_route(),
            dns_hijack: default_dns_hijack(),
        }
    }
}

pub(crate) fn default_tun_stack() -> String {
    "mixed".to_string()
}
fn default_tun_auto_route() -> bool {
    true
}
fn default_tun_auto_detect_interface() -> bool {
    true
}
pub(crate) fn default_tun_strict_route() -> bool {
    true
}
fn default_dns_hijack() -> Vec<String> {
    vec!["any:53".to_string(), "tcp://any:53".to_string()]
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

    /// DoH 端点列表（自定义 DNS-over-HTTPS 上游）。
    /// 空 = 使用内置默认（阿里/腾讯 DoH）。仅当 nameserver 包含 https:// 时生效。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub doh_endpoints: Vec<String>,

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
            doh_endpoints: default_dns_doh_endpoints(),
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
/// 自定义 DoH 端点（空 = 使用内置默认）。配置示例：
/// doh-endpoints:
///   - https://dns.google/dns-query
///   - https://cloudflare-dns.com/dns-query
fn default_dns_doh_endpoints() -> Vec<String> {
    Vec::new()
}
/// proxy-server-nameserver：解析代理节点服务器域名时使用的明文 DNS。
/// mihomo 要求该字段必须是明文 IP（非 DoH/DoT），否则会报错拒绝启动。
fn default_dns_proxy_server_nameserver() -> Vec<String> {
    vec!["223.5.5.5".to_string(), "119.29.29.29".to_string()]
}

/// --- AdvancedConfig ---
///
/// 只保留真正被后端消费的字段。历史上的 disable-commit-animation / log-format /
/// explicit-proxy / connect|read|write-timeout / geox-url 从未接入任何逻辑
/// （审计 B9），已删除；旧配置里的这些键反序列化时被忽略，不影响加载。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub struct AdvancedConfig {
    /// GeoIP 自定义下载地址（空 = 使用内置源）。仅作用于手动 / 自动 GeoData 更新。
    #[serde(default)]
    pub geoip_url: String,

    /// GeoSite 自定义下载地址（空 = 使用内置源）
    #[serde(default)]
    pub geosite_url: String,

    /// mihomo `tcp-concurrent`（TCP 并发连接）：同时向全部解析出的 IP 发起连接、
    /// 取最快者，可显著降低多出口 / 跨境高延迟链路的建连时间。默认 false
    /// 保持现网行为；开关变更直接写入运行时配置（B9：每个开关必须有
    /// 变更→运行时可观察效果，见 build_runtime_config）。
    #[serde(default)]
    pub tcp_concurrent: bool,

    /// mihomo `unified-delay`（统一延迟计量）：url-test 延迟去掉握手差异
    /// （RTT×2 计量），不同协议间横向可比。默认 false 保持现网行为；
    /// 不调整 url-test interval（300s）/ tolerance（100ms）——上游压力红线。
    #[serde(default)]
    pub unified_delay: bool,
}

/// 内置规则集名称（顺序无关）。
/// 单一来源：shared/product.json（构建期由 build.rs 生成 product_data.rs，D4-4），
/// 与 Android 端共享同一份数据文件，禁止在本文件手写字面量副本。
pub const BUILTIN_RULE_SETS: &[&str] = product_data::BUILTIN_RULE_SETS_SHARED;

/// 生成于 build.rs 的产品级共享常量（组名 / 规则集名 / 规则链模板）。
/// 供 config::model 与 core::config（运行时配置生成）、util::normalizer（保留名）消费。
pub(crate) mod product_data {
    include!(concat!(env!("OUT_DIR"), "/product_data.rs"));
}

/// 内置规则提供者：默认基线的 rule-providers 定义。
///
/// 一律是本地 `type: file`（`./rules/<name>.yaml`，相对 mihomo 的 `-d` 目录 Data/）：
/// 随包附带这些文件，离线也能加载；刷新由 `geodata::rules` 的**签名清单更新器**
/// 负责（验签 + 逐文件 SHA256 + 回滚保护），不再让 mihomo 自行从 `main` 分支
/// 浮动拉取未经校验的内容（审计 A4）。
pub fn default_rule_providers() -> HashMap<String, serde_norway::Value> {
    BUILTIN_RULE_SETS
        .iter()
        .map(|name| (name.to_string(), builtin_rule_provider(name)))
        .collect()
}

fn builtin_rule_provider(name: &str) -> serde_norway::Value {
    serde_norway::from_str(&format!(
        "type: file\nbehavior: classical\npath: ./rules/{}.yaml\n",
        name
    ))
    .unwrap_or_default()
}

/// 把旧配置里持久化的内置 `type: http` 规则集（指向外部仓库 main 分支）迁移为本地
/// `type: file`。用户自定义的其它规则集、以及指向别处的同名 provider 保持原样。
/// 返回是否发生了迁移。
pub fn migrate_builtin_rule_providers(
    providers: &mut HashMap<String, serde_norway::Value>,
) -> bool {
    let mut changed = false;
    for name in BUILTIN_RULE_SETS {
        let Some(v) = providers.get(*name) else {
            providers.insert(name.to_string(), builtin_rule_provider(name));
            changed = true;
            continue;
        };
        let is_old_http = v.get("type").and_then(|t| t.as_str()) == Some("http")
            && v.get("url").and_then(|u| u.as_str()).is_some_and(|u| {
                u.starts_with("https://raw.githubusercontent.com/akaspyrean/external/")
            });
        if is_old_http {
            providers.insert(name.to_string(), builtin_rule_provider(name));
            changed = true;
        }
    }
    changed
}

/// 内置代理组：默认基线的 proxy-groups 段（规格固定六组）。
/// - 扶梯出行：[自动优选, 人工优选, DIRECT]（默认自动，流量走低延迟节点）
/// - 人工智能 / 影音视听：[人工优选, 自动优选, DIRECT]（默认手动）
/// - 人工优选（select）：全部有效节点，纯手动选择（无任何自动成员）
/// - 自动优选（url-test）：全部有效节点，自动选延迟最低者
///
/// 叶子组初始为空（无订阅时无可用节点），订阅导入后由
/// `build_runtime_config` 注入真实节点名。DIRECT 为路由组兜底项。
/// GLOBAL 为全局模式专用组：`mode: global` 时所有流量走它，只含 DIRECT/REJECT
/// 与两个叶子组；代理页面按模式联动（rule 隐藏 / global 独占显示 / direct 无组），
/// 由前端 `ProxiesView` 实现，托盘保留以便切换。
///
/// 注意：`自动优选` 是 `url-test` 类型，mihomo 仅对真实代理节点做延迟测速，
/// DIRECT 不是代理节点——注入 DIRECT 会让 url-test 把直连当作"零延迟最优节点"
/// 永久霸占自动组，所有真实节点永远拿不到流量。故订阅注入时只放真实节点名。
/// mihomo（v1.19.x）拒绝 proxies 为空的代理组，无订阅时的空列表由
/// `build_runtime_config` 在生成运行时配置时补 DIRECT 占位（仅零节点状态）。
pub fn default_proxy_groups() -> Vec<serde_norway::Value> {
    // 组名来自 shared/product.json（build.rs 生成），与 Android 端同源。
    let yaml = format!(
        r#"
- name: {global}
  type: select
  proxies: [DIRECT, REJECT, {manual}, {auto}]
- name: {ladder}
  type: select
  proxies: [{auto}, {manual}, DIRECT]
- name: {ai}
  type: select
  proxies: [{manual}, {auto}, DIRECT]
- name: {media}
  type: select
  proxies: [{manual}, {auto}, DIRECT]
- name: {manual}
  type: select
  proxies: []
- name: {auto}
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
"#,
        global = product_data::GROUP_GLOBAL,
        ladder = product_data::GROUP_LADDER,
        ai = product_data::GROUP_AI,
        media = product_data::GROUP_MEDIA,
        manual = product_data::GROUP_MANUAL,
        auto = product_data::GROUP_AUTO,
    );
    serde_norway::from_str(&yaml).unwrap_or_default()
}

/// 内置代理组迁移：把持久化配置里的六个内置组重置为当前基线定义。
/// 内置组骨架（成员序 / 类型 / 测速参数）由应用所有，UI 不开放组成员编辑；
/// 旧版本持久化的过时骨架（如 扶梯出行=[人工优选, 自动优选]）会使默认选中项
/// 偏离规格（扶梯出行应默认自动优选），一律以当前 `default_proxy_groups` 覆盖。
/// 同名自定义组不受影响；缺失的内置组补回列表尾部。返回是否发生了迁移。
pub fn migrate_builtin_proxy_groups(groups: &mut Vec<serde_norway::Value>) -> bool {
    let mut changed = false;
    for d in default_proxy_groups() {
        let Some(name) = d.get("name").and_then(|n| n.as_str()).map(str::to_string) else {
            continue;
        };
        match groups
            .iter_mut()
            .find(|g| g.get("name").and_then(|n| n.as_str()) == Some(name.as_str()))
        {
            Some(slot) => {
                if *slot != d {
                    *slot = d;
                    changed = true;
                }
            }
            None => {
                groups.push(d);
                changed = true;
            }
        }
    }
    changed
}

/// 内置路由规则：内置基线的分流规则段。
/// 采用经典分流：国内直连、广告拦截、AI/影音走专属组、其余（proxy 规则集 + MATCH）
/// 走主组。广告拦截双保险：内置 ad 规则集（external/ad.yaml，约 21 万条）
/// 优先于 GEOSITE category-ads-all 兜底，确保离线也有基础拦截。
/// 规则链模板来自 shared/product.json（单一来源），组名占位符在构建时替换。
pub fn default_rules() -> Vec<String> {
    use product_data as pd;
    pd::RULES_CHAIN_TEMPLATES
        .iter()
        .map(|tpl| {
            tpl.replace("{GROUP_AI}", pd::GROUP_AI)
                .replace("{GROUP_MEDIA}", pd::GROUP_MEDIA)
                .replace("{GROUP_LADDER}", pd::GROUP_LADDER)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 旧版持久化骨架迁移：扶梯出行回到 [自动优选, 人工优选, DIRECT]（默认自动），
    /// 缺失的内置组补回，同名自定义组不受影响，且迁移幂等。
    #[test]
    fn migrate_builtin_proxy_groups_restores_current_baseline() {
        let mut groups: Vec<serde_norway::Value> = serde_norway::from_str(
            r#"
- name: 扶梯出行
  type: select
  proxies: [人工优选, 自动优选]
- name: 自定义组
  type: select
  proxies: [节点A]
"#,
        )
        .expect("parse fixture");

        assert!(migrate_builtin_proxy_groups(&mut groups));

        let get = |name: &str| -> Vec<String> {
            groups
                .iter()
                .find(|g| g.get("name").and_then(|n| n.as_str()) == Some(name))
                .map(|g| {
                    g.get("proxies")
                        .and_then(|p| p.as_sequence())
                        .map(|s| {
                            s.iter()
                                .filter_map(|v| v.as_str().map(str::to_string))
                                .collect()
                        })
                        .unwrap_or_default()
                })
                .unwrap_or_default()
        };

        // 扶梯出行：成员序按规格，默认首项=自动优选
        assert_eq!(get("扶梯出行"), vec!["自动优选", "人工优选", "DIRECT"]);
        // 缺失的内置组补回（含 人工智能 / 影音视听 / GLOBAL / 人工优选 / 自动优选）
        assert_eq!(get("人工智能"), vec!["人工优选", "自动优选", "DIRECT"]);
        assert_eq!(get("影音视听"), vec!["人工优选", "自动优选", "DIRECT"]);
        assert!(groups
            .iter()
            .any(|g| g.get("name").and_then(|n| n.as_str()) == Some("GLOBAL")));
        // 同名自定义组不被触碰
        assert_eq!(get("自定义组"), vec!["节点A"]);
        // 幂等：再跑一遍不再变化
        assert!(!migrate_builtin_proxy_groups(&mut groups));
    }

    /// 旧版 config.yaml：external-controller / secret 位于顶层。
    /// 往返后必须保留原始值，且仍写回顶层（mihomo 可识别），
    /// 不得丢失、不得空串、不得错位嵌套到 proxy: 下。
    #[test]
    fn roundtrip_preserves_top_level_external_controller() {
        let legacy_yaml = r#"
mixed-port: 7890
allow-lan: false
external-controller: 127.0.0.1:50715
secret: b1616fdd-63a8-44e9-b196-c63b68307a9b
log-level: error
"#;

        let config: Config = serde_norway::from_str(legacy_yaml).expect("parse legacy yaml");
        assert_eq!(config.proxy.external_controller, "127.0.0.1:50715");
        assert_eq!(config.proxy.secret, "b1616fdd-63a8-44e9-b196-c63b68307a9b");

        let yaml = serde_norway::to_string(&config).expect("serialize");
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
        let again: Config = serde_norway::from_str(&yaml).expect("re-parse round-tripped yaml");
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

        let config: Config = serde_norway::from_str(yaml).expect("parse");
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
        let out = serde_norway::to_string(&config).expect("serialize");
        let again: Config = serde_norway::from_str(&out).expect("re-parse");
        let proxies = again.extra.get("proxies").unwrap().as_sequence().unwrap();
        assert_eq!(proxies.len(), 1);
        assert_eq!(
            proxies[0].get("name").and_then(|n| n.as_str()),
            Some("Node1")
        );
        assert!(again.extra.contains_key("hosts"));
    }

    /// 旧配置键 `proxy-mode` 必须仍可被解析（alias 兼容旧格式）。
    #[test]
    fn legacy_proxy_mode_alias_parses() {
        let yaml = "proxy-mode: global\nmixed-port: 7890\n";
        let config: Config = serde_norway::from_str(yaml).expect("parse alias");
        assert_eq!(config.general.proxy_mode, "global");
        // 序列化应写出新键 `mode`，而非旧的 `proxy-mode`
        let out = serde_norway::to_string(&config).expect("serialize");
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
        let config: Config = serde_norway::from_str(yaml).expect("parse bool geodata-mode");
        assert_eq!(config.general.geodata_mode.as_bool(), Some(true));

        let out = serde_norway::to_string(&config).expect("serialize");
        let again: Config = serde_norway::from_str(&out).expect("re-parse");
        assert_eq!(again.general.geodata_mode.as_bool(), Some(true));

        // 应用级字符串值
        let yaml2 = "geodata-mode: manual\n";
        let config2: Config = serde_norway::from_str(yaml2).expect("parse string geodata-mode");
        assert_eq!(config2.general.geodata_mode.as_str(), Some("manual"));
    }

    /// 全新默认配置的控制器默认值应为真实地址/密钥，而非空串。
    #[test]
    fn default_proxy_has_controller_defaults() {
        let config = Config::default();
        assert_eq!(config.proxy.external_controller, "127.0.0.1:9090");
        assert_eq!(config.proxy.secret, "clash-edge-secret");
    }

    /// A/B/C：TUN 默认值面向普通 Windows 用户——stack=mixed、auto-route=true、
    /// auto-detect-interface=true（开箱即用），enable 默认关闭。
    #[test]
    fn tun_default_stack_is_mixed() {
        let tun = Config::default().tun;
        assert_eq!(tun.stack, "mixed");
        assert!(tun.auto_route, "auto-route must default on");
        assert!(
            tun.auto_detect_interface,
            "auto-detect-interface must default on"
        );
        assert!(!tun.enable, "tun must default off");
        assert_eq!(tun.interface_name, None);
    }

    /// D：默认 dns-hijack 接管本机 DNS → any:53 + tcp://any:53（TUN 内核不会漏 DNS）。
    #[test]
    fn tun_default_dns_hijack() {
        assert_eq!(
            TunConfig::default().dns_hijack,
            vec!["any:53".to_string(), "tcp://any:53".to_string()]
        );
    }

    /// 旧配置（/老版本/导入）缺 dns-hijack 字段时反序列化自动补默认，
    /// 保证 TUN runtime 生成非法；
    /// 已有合法 dns-hijack 的用户配置必须原样保留（不被默认覆盖）。
    #[test]
    fn tun_missing_dns_hijack_fills_default_but_preserves_existing() {
        // 缺字段：填充默认
        let config: TunConfig = serde_norway::from_str("enable: true\nstack: mixed\n").unwrap();
        assert_eq!(
            config.dns_hijack,
            vec!["any:53".to_string(), "tcp://any:53".to_string()]
        );

        // 已有自定义值：保留
        let config: TunConfig =
            serde_norway::from_str("enable: true\nstack: gvisor\ndns-hijack:\n  - 1.1.1.1:53\n")
                .unwrap();
        assert_eq!(config.dns_hijack, vec!["1.1.1.1:53".to_string()]);
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
        let yaml = serde_norway::to_string(&config).unwrap();
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

    // ---- D5: advanced.tcp-concurrent / unified-delay ----

    /// 默认值必须为 false（保持现网行为；B9 规则：开关必须有运行时可观察效果）。
    #[test]
    fn advanced_defaults_are_false() {
        let cfg = Config::default();
        assert!(!cfg.advanced.tcp_concurrent);
        assert!(!cfg.advanced.unified_delay);
    }

    /// kebab-case 往返：设置页保存 `tcp-concurrent: true` / `unified-delay: true`
    /// 后重启必须保留（不能因 serde 键名错位而静默丢失）。
    #[test]
    fn advanced_switches_roundtrip_kebab_case() {
        let yaml = r#"
tcp-concurrent: true
unified-delay: true
advanced:
  tcp-concurrent: true
  unified-delay: true
"#;
        let cfg: Config = serde_norway::from_str(yaml).expect("parse advanced switches");
        assert!(cfg.advanced.tcp_concurrent);
        assert!(cfg.advanced.unified_delay);

        let out = serde_norway::to_string(&cfg).expect("serialize");
        assert!(
            out.contains("tcp-concurrent: true"),
            "must serialize tcp-concurrent:\n{}",
            out
        );
        assert!(
            out.contains("unified-delay: true"),
            "must serialize unified-delay:\n{}",
            out
        );
        // 再解析一次，保证幂等往返
        let again: Config = serde_norway::from_str(&out).expect("re-parse");
        assert!(again.advanced.tcp_concurrent);
        assert!(again.advanced.unified_delay);
    }

    /// 旧配置缺这两个字段时反序列化自动补默认 false（不炸、不误开）。
    #[test]
    fn advanced_missing_switches_default_false() {
        let cfg: Config = serde_norway::from_str("mixed-port: 7890\n").expect("parse legacy");
        assert!(!cfg.advanced.tcp_concurrent);
        assert!(!cfg.advanced.unified_delay);
    }
}
