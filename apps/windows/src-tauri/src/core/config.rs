// src-tauri/src/core/config.rs
//! 配置加载/保存 与 运行时配置（MihomoConfig）生成
//!
//! AppConfig（Data/config.yaml，应用单一数据源）与 MihomoConfig
//! （Data/runtime-config.yaml，mihomo 实际加载）分离：
//! - AppConfig 完整保留应用级字段与订阅未知键（`#[serde(flatten)]` 兜底）；
//! - 启动 / 重载时由 `build_runtime_config` 从「AppConfig + 激活 Profile 内容」
//!   生成只含 mihomo 顶层合法键的运行时配置，以 `-f` 交给 mihomo。

use tracing::{info, warn};

use crate::config::model::Config;
use crate::util::error::{Error, Result};

/// mihomo 顶层 `mode` 合法值。
/// 官方 config.yaml 模板仅 rule / global / direct 三值；
/// `script` 是 Clash Premium 的遗留，mihomo 不接受。
const VALID_PROXY_MODES: &[&str] = &["rule", "global", "direct"];

/// mihomo `find-process-mode` 合法值。
/// 官方模板注释：`# find-process-mode has 3 values: always, strict, off`。
const VALID_FIND_PROCESS_MODES: &[&str] = &["off", "strict", "always"];

/// mihomo `tun.stack` 合法值（仅这三值；不得增加其他 stack 类型）。
/// 非法值归一为 `mixed`（对普通 Windows 用户优先原生驱动、复杂场景回退用户态栈）。
const VALID_TUN_STACKS: &[&str] = &["mixed", "system", "gvisor"];

/// mihomo `log-level` 合法值（官方模板：debug / info / warning / error / silent）
const VALID_LOG_LEVELS: &[&str] = &["debug", "info", "warning", "error", "silent"];

/// 订阅内容允许透传到运行时配置的顶层键白名单。
///
/// 设计原则：订阅默认仅提供代理节点。应用采用内置代理组骨架（GLOBAL + 5 组）
/// 与内置规则链，订阅自带的 proxy-groups/rules/rule-providers/proxy-providers/
/// script/hosts/sniffer/listeners/external-controller/external-ui/dns/tun 等一律
/// 不透传——防止订阅任意改写控制器/绕过应用分流结构/注入恶意 hosts 或 sniffer。
///
/// 仅 `proxies`（真实代理节点列表）允许从订阅进入运行时配置，由 step 5 注入
/// 内置叶子组。
const PROFILE_ALLOWED_KEYS: &[&str] = &["proxies"];

/// AppConfig.extra 允许透传到运行时配置的顶层键白名单。
///
/// `extra` 兜底键来自用户导入的完整 mihomo 配置（import_config）。与订阅不同，
/// 导入是用户显式行为，但仍有未知顶层字段不得无差别透传的要求。`proxies` 由
/// step 5 统一处理；其余键（proxy-providers/hosts/sniffer/script/listeners 等）
/// 不透传，防止导入配置携带的恶意 hosts/sniffer/script 进入运行时。
const EXTRA_ALLOWED_KEYS: &[&str] = &[];

/// 应用级 geodata-mode 值（描述 GeoData 更新来源，与 mihomo 的 geodata-mode
/// 语义不同，这些值不会写给 mihomo）。
const APP_GEODATA_MODES: &[&str] = &["manual", "use-external", "remote"];

/// 内置域名嗅探配置（`general.sniffer` 开启时写入运行时配置）。
/// `override-destination: false`：只用嗅探结果做规则匹配，不改写连接目标，
/// 对不支持域名目标的应用最保守。
const SNIFFER_YAML: &str = r#"
enable: true
force-dns-mapping: true
parse-pure-ip: true
override-destination: false
sniff:
  TLS:
    ports: [443, 8443]
  HTTP:
    ports: [80, 8080-8880]
  QUIC:
    ports: [443, 8443]
"#;

/// 合并配置规则 - 运行时语义校验与归一：
/// - 校验并修正代理模式（rule / global / direct）
/// - 校验 geodata_mode（应用级值，仅空串回退 manual，不覆盖 mihomo 语义值）
/// - 校验 find_process_mode（off / strict / always）
/// - 确保默认配置文件存在
///
/// 在 `ConfigManager::init`（加载）与测试中调用，保证运行时配置永远合法。
pub(crate) fn merge_rules(config: Config) -> Config {
    let mut config = config;

    // 确保 proxy 模式有效：mihomo 只接受 rule / global / direct。
    if config.general.proxy_mode.is_empty()
        || !VALID_PROXY_MODES.contains(&config.general.proxy_mode.as_str())
    {
        config.general.proxy_mode = "rule".to_string();
        warn!("Invalid proxy mode, defaulting to 'rule'");
    }

    // 确保 geodata_mode 有效：仅处理应用级值。mihomo 语义值
    // （bool / "metax" / "v2ray"）保持原样，避免覆盖导入配置里的真实设置。
    if let Some(s) = config.general.geodata_mode.as_str() {
        if s.is_empty() {
            config.general.geodata_mode = crate::config::model::default_geodata_mode();
            warn!("Invalid geodata_mode, defaulting to 'manual'");
        }
    }

    // 确保 find_process_mode 有效：mihomo 只接受 off / strict / always。
    if config.general.find_process_mode.is_empty()
        || !VALID_FIND_PROCESS_MODES.contains(&config.general.find_process_mode.as_str())
    {
        config.general.find_process_mode = "off".to_string();
        warn!("Invalid find_process_mode, defaulting to 'off'");
    }

    // TUN stack 校验：mihomo 只接受 mixed / system / gvisor。
    // 非法值（空、未知、旧值）一律回退到默认 mixed，保证 TUN runtime 永不非法。
    if config.tun.stack.is_empty() || !VALID_TUN_STACKS.contains(&config.tun.stack.as_str()) {
        config.tun.stack = crate::config::model::default_tun_stack();
        warn!("Invalid tun stack, defaulting to 'mixed'");
    }

    // 规则提供者：旧版持久化的内置 http provider（浮动拉取 main 分支）迁移为本地 file
    if crate::config::model::migrate_builtin_rule_providers(&mut config.rule_providers) {
        info!("Builtin rule-providers migrated to local files (signed updater)");
    }

    // 确保默认配置文件存在
    if config.general.profile.is_empty() {
        config.general.profile = "DIRECT".to_string();
    }

    config
}

/// 生成 mihomo 运行时配置（MihomoConfig）。
///
/// 输入：AppConfig（应用设置，单一数据源）+ 激活 Profile 的原始 YAML 内容。
/// 输出：只含 mihomo 顶层合法键的 YAML 值，供写入 runtime-config.yaml 并以
/// `-f` 交给 mihomo 加载。应用级字段（profile / locale / mixin-enabled /
/// advanced / profiles / geo-auto-update / geodata-mode 应用值）不进入运行时。
///
/// 合并策略（应用控制运行时结构，订阅只提供节点）：
/// 1. AppConfig 控制运行时关键设置（端口 / 控制器 / 模式 / TUN / DNS），订阅不得覆盖；
/// 2. AppConfig.extra 兜底键作为基线透传（用户自行导入的自定义键）；
/// 3. 激活 Profile 只提供节点：应用始终采用内置组骨架（GLOBAL + 5 组）与内置规则链，
///    订阅节点名强制注入叶子组（人工优选只含真实节点 / 自动优选含 DIRECT 兜底）；订阅自带的 proxy-groups/rules 不采用；
/// 4. rule-providers：AppConfig 内置 5 组为底，订阅同名覆盖；
/// 5. 订阅顶层键采用严格白名单（仅 `proxies`），其余键（hosts / sniffer /
///    proxy-providers / script / listeners 等）一律不透传——防止订阅改写
///    控制器、注入 hosts/sniffer 或绕过应用分流结构。
pub fn build_runtime_config(
    app: &Config,
    profile_content: Option<&str>,
    region_map: Option<&std::collections::HashMap<String, String>>,
) -> Result<serde_yaml::Value> {
    let mut map = serde_yaml::Mapping::new();
    macro_rules! put {
        ($k:expr, $v:expr) => {
            map.insert(serde_yaml::Value::String($k.to_string()), $v);
        };
    }

    // 1) AppConfig 受控运行时设置
    put!(
        "mixed-port",
        serde_yaml::Value::from(app.general.mixed_port)
    );
    put!("allow-lan", serde_yaml::Value::from(app.general.allow_lan));
    // allow-lan 高级控制。仅 allow-lan=true 时写入：
    // - bind-address：限定监听接口（如仅内网 IP）；
    // - lan-allowed-ips：来源网段白名单（CIDR）。
    // 值为空/None 时不写键，保持 mihomo 默认行为；非法值（含空白/控制符
    // 的 bind-address、非 CIDR 形态的网段）直接丢弃，不进入运行时配置。
    if app.general.allow_lan {
        let bind = app.general.bind_address.as_deref().unwrap_or("").trim();
        let bind_valid = !bind.is_empty()
            && (bind == "*"
                || bind.parse::<std::net::IpAddr>().is_ok()
                || (!bind.chars().any(|c| c.is_whitespace() || c.is_control())
                    && !bind.contains('|')));
        if bind_valid {
            put!("bind-address", serde_yaml::Value::from(bind.to_string()));
        }
        let allowed: Vec<String> = app
            .general
            .lan_allowed_ips
            .iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .filter(|s| s.parse::<ipnet::IpNet>().is_ok() || s.parse::<std::net::IpAddr>().is_ok())
            .collect();
        if !allowed.is_empty() {
            put!("lan-allowed-ips", serde_yaml::to_value(&allowed)?);
        }
    }
    put!(
        "mode",
        serde_yaml::Value::from(app.general.proxy_mode.clone())
    );
    // 日志级别：设置页的 log-level 真正写入运行时配置，
    // 不再固定 info。非法值/空值归一到 info（mihomo 官方模板默认值）。
    // 注意 silent/error 会让内置日志页几乎无输出——这是用户显式选择，
    // UI 侧应有相应提示，后端不再越权改写用户意图。
    let log_level = {
        let l = app.general.log_level.trim().to_ascii_lowercase();
        if VALID_LOG_LEVELS.contains(&l.as_str()) {
            l
        } else {
            "info".to_string()
        }
    };
    put!("log-level", serde_yaml::Value::from(log_level));
    put!("ipv6", serde_yaml::Value::from(app.general.ipv6));
    put!(
        "find-process-mode",
        serde_yaml::Value::from(app.general.find_process_mode.clone())
    );
    put!(
        "external-controller",
        serde_yaml::Value::from(app.proxy.external_controller.clone())
    );
    put!("secret", serde_yaml::Value::from(app.proxy.secret.clone()));
    put!("tun", serde_yaml::to_value(&app.tun)?);
    // 域名嗅探（可选）：应用自己的 sniffer 配置；订阅里的 sniffer 仍一律不透传。
    if app.general.sniffer {
        put!(
            "sniffer",
            serde_yaml::from_str::<serde_yaml::Value>(SNIFFER_YAML)?
        );
    }
    put!("dns", serde_yaml::to_value(&app.dns)?);

    // geodata-mode：仅透传 mihomo 语义值（bool / "metax" / "v2ray"）；
    // 应用级值（manual / use-external / remote）与空值不写给 mihomo。
    let geodata_is_app_level = app
        .general
        .geodata_mode
        .as_str()
        .is_some_and(|s| APP_GEODATA_MODES.contains(&s));
    let geodata_is_empty = app
        .general
        .geodata_mode
        .as_str()
        .is_some_and(|s| s.is_empty());
    if !geodata_is_app_level && !geodata_is_empty {
        put!("geodata-mode", app.general.geodata_mode.clone());
    }

    // 2) AppConfig.extra 兜底键：仅白名单内键透传（防止导入配置携带的
    //    hosts/sniffer/script/proxy-providers 等未知顶层字段进入运行时配置）。
    //    `proxies` 由 step 5 统一处理，不在此重复透传。
    for (k, v) in &app.extra {
        let Some(s) = k.as_str() else { continue };
        if EXTRA_ALLOWED_KEYS.contains(&s) {
            map.insert(k.clone(), v.clone());
        }
    }

    // 3) 解析激活 Profile 内容（空/缺失 → 空映射）
    let profile_map = match profile_content {
        Some(c) if !c.trim().is_empty() => {
            let value: serde_yaml::Value = serde_yaml::from_str(c)
                .map_err(|e| Error::ConfigParse(format!("Invalid profile YAML: {}", e)))?;
            value.as_mapping().cloned().unwrap_or_default()
        }
        _ => serde_yaml::Mapping::new(),
    };
    // 空 `proxies:` 列表视为"未提供"：Some([]) 会遮蔽 AppConfig.extra 里
    // 用户通过「导入配置」显式带入的节点，导致导入后运行时拿零节点
    // （全部直连），表现为"导入配置不起效"。
    let profile_proxies = profile_map
        .get("proxies")
        .and_then(|v| v.as_sequence())
        .filter(|s| !s.is_empty())
        .cloned();

    // 4) 订阅仅提供代理节点：profile 顶层键只透传白名单（`proxies`），
    //    其余一律忽略（rules/proxy-groups/rule-providers/proxy-providers/
    //    script/hosts/sniffer/listeners/external-controller/external-ui/dns/tun
    //    等均不透传——应用采用内置分流结构，订阅不得改写控制器或注入 hosts）。
    for (k, v) in &profile_map {
        let Some(key) = k.as_str() else { continue };
        if !PROFILE_ALLOWED_KEYS.contains(&key) {
            continue;
        }
        // `proxies` 保留到 step 5 注入内置叶子组
        if key == "proxies" {
            continue;
        }
        map.insert(k.clone(), v.clone());
    }

    // 5) proxies / proxy-groups / rules：
    //    应用始终采用内置组骨架（GLOBAL + 5 组）与内置规则链。订阅只提供节点。
    //    有区域信息时按区域动态分组：
    //    - 自动优选（url-test）：全部节点测速选最低延迟（不变）
    //    - 人工优选（select）：如含美国节点 -> [美国优选(url-test), 美国节点...],
    //      默认=美国优选=最低延迟美国节点, 用户可手选任意美国节点;
    //      无美国节点时降级为 [自动优选, 全部节点...].
    //    - 美国优选 + 各非美国区域组（香港优选/日本优选/...）动态生成,
    //      追加到 GLOBAL 与 扶梯出行 选项尾部.
    //    无区域信息（旧 profile/无 GeoIP.dat/DoH 失败）-> 退化为当前行为.
    let mut groups = app.proxy_groups.clone();
    let effective_proxies = profile_proxies.or_else(|| {
        app.extra
            .get("proxies")
            .and_then(|v| v.as_sequence())
            .filter(|s| !s.is_empty())
            .cloned()
    });
    if let Some(proxies) = &effective_proxies {
        let node_names: Vec<String> = proxies
            .iter()
            .filter_map(|p| p.get("name").and_then(|n| n.as_str()).map(str::to_string))
            .collect();
        if !node_names.is_empty() {
            let us_nodes = partition_by_region(&node_names, region_map, "US");
            let (region_groups, us_group_name) =
                build_region_groups(&node_names, region_map, &us_nodes);

            for group in groups.iter_mut() {
                let Some(gmap) = group.as_mapping_mut() else {
                    continue;
                };
                let Some(gname) = gmap
                    .get("name")
                    .and_then(|n| n.as_str())
                    .map(str::to_string)
                else {
                    continue;
                };

                if gname == "自动优选" {
                    if let Some(plist) = gmap.get_mut("proxies").and_then(|p| p.as_sequence_mut()) {
                        plist.clear();
                        for n in &node_names {
                            plist.push(serde_yaml::Value::from(n.clone()));
                        }
                    }
                } else if gname == "人工优选" {
                    if let Some(plist) = gmap.get_mut("proxies").and_then(|p| p.as_sequence_mut()) {
                        plist.clear();
                        if !us_nodes.is_empty() {
                            plist.push(serde_yaml::Value::from(
                                us_group_name
                                    .clone()
                                    .unwrap_or_else(|| "美国优选".to_string()),
                            ));
                            for n in &us_nodes {
                                plist.push(serde_yaml::Value::from(n.clone()));
                            }
                        } else {
                            plist.push(serde_yaml::Value::from("自动优选"));
                            for n in &node_names {
                                plist.push(serde_yaml::Value::from(n.clone()));
                            }
                        }
                    }
                } else if (gname == "GLOBAL" || gname == "扶梯出行") && !region_groups.is_empty()
                {
                    if let Some(plist) = gmap.get_mut("proxies").and_then(|p| p.as_sequence_mut()) {
                        for rg in &region_groups {
                            let gname_rg = rg.get("name").and_then(|n| n.as_str()).unwrap_or("");
                            if !plist.iter().any(|v| v.as_str() == Some(gname_rg)) {
                                plist.push(serde_yaml::Value::from(gname_rg.to_string()));
                            }
                        }
                    }
                }
            }
            for rg in region_groups {
                groups.push(rg);
            }
        }
        put!("proxies", serde_yaml::to_value(proxies)?);
    }

    // 6) 零节点兜底：mihomo 拒绝 proxies 为空的代理组。
    //    自动优选是 url-test 组，零节点时删除整组并从引用中剔除；
    //    空的动态区域组同样删除；人工优选（select）补 DIRECT 兜底。
    let mut drop_auto_group = false;
    for group in groups.iter_mut() {
        let Some(gmap) = group.as_mapping_mut() else {
            continue;
        };
        let Some(gname) = gmap
            .get("name")
            .and_then(|n| n.as_str())
            .map(str::to_string)
        else {
            continue;
        };
        if gname == "自动优选" {
            let is_empty = gmap
                .get("proxies")
                .and_then(|p| p.as_sequence())
                .map(|s| s.is_empty())
                .unwrap_or(true);
            if is_empty {
                drop_auto_group = true;
            }
        }
    }
    if drop_auto_group {
        groups.retain(|g| {
            let name = g.get("name").and_then(|n| n.as_str()).unwrap_or("");
            if name == "自动优选" {
                return false;
            }
            if name.ends_with("优选") && name != "人工优选" {
                let is_empty = g
                    .get("proxies")
                    .and_then(|p| p.as_sequence())
                    .map(|s| s.is_empty())
                    .unwrap_or(true);
                if is_empty {
                    return false;
                }
            }
            true
        });
        for group in groups.iter_mut() {
            let Some(gmap) = group.as_mapping_mut() else {
                continue;
            };
            if let Some(plist) = gmap.get_mut("proxies").and_then(|p| p.as_sequence_mut()) {
                plist.retain(|v| v.as_str() != Some("自动优选"));
            }
            let Some(gname) = gmap
                .get("name")
                .and_then(|n| n.as_str())
                .map(str::to_string)
            else {
                continue;
            };
            if gname == "人工优选" {
                let is_empty = gmap
                    .get("proxies")
                    .and_then(|p| p.as_sequence())
                    .map(|s| s.is_empty())
                    .unwrap_or(true);
                if is_empty {
                    gmap.insert(
                        serde_yaml::Value::from("proxies"),
                        serde_yaml::Value::Sequence(vec![serde_yaml::Value::from("DIRECT")]),
                    );
                }
            }
        }
    }
    put!("proxy-groups", serde_yaml::to_value(groups)?);
    put!("rules", serde_yaml::to_value(app.rules.clone())?);

    // rule-providers 兜底：订阅未提供时用 AppConfig 内置 5 组
    if !map.contains_key("rule-providers") {
        put!(
            "rule-providers",
            serde_yaml::to_value(app.rule_providers.clone())?
        );
    }

    Ok(serde_yaml::Value::Mapping(map))
}

fn region_display_name(code: &str) -> String {
    match code {
        "US" => "美国",
        "HK" => "香港",
        "TW" => "台湾",
        "JP" => "日本",
        "SG" => "新加坡",
        "KR" => "韩国",
        "MY" => "马来西亚",
        "TH" => "泰国",
        "PH" => "菲律宾",
        "VN" => "越南",
        "ID" => "印度尼西亚",
        "IN" => "印度",
        "TR" => "土耳其",
        "AE" => "阿联酋",
        "RU" => "俄罗斯",
        "DE" => "德国",
        "GB" => "英国",
        "FR" => "法国",
        "NL" => "荷兰",
        "SE" => "瑞典",
        "CH" => "瑞士",
        "ES" => "西班牙",
        "IT" => "意大利",
        "CA" => "加拿大",
        "MX" => "墨西哥",
        "BR" => "巴西",
        "AR" => "阿根廷",
        "AU" => "澳大利亚",
        "NZ" => "新西兰",
        "ZA" => "南非",
        "EG" => "埃及",
        "SA" => "沙特",
        "IL" => "以色列",
        "KH" => "柬埔寨",
        "PK" => "巴基斯坦",
        "BD" => "孟加拉",
        "KZ" => "哈萨克斯坦",
        _ => code,
    }
    .to_string()
}

fn partition_by_region(
    node_names: &[String],
    region_map: Option<&std::collections::HashMap<String, String>>,
    code: &str,
) -> Vec<String> {
    match region_map {
        Some(map) => node_names
            .iter()
            .filter(|n| map.get(*n).map(|c| c == code).unwrap_or(false))
            .cloned()
            .collect(),
        None => Vec::new(),
    }
}

fn build_region_groups(
    node_names: &[String],
    region_map: Option<&std::collections::HashMap<String, String>>,
    us_nodes: &[String],
) -> (Vec<serde_yaml::Value>, Option<String>) {
    let Some(rmap) = region_map else {
        return (Vec::new(), None);
    };

    let mut by_region: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for n in node_names {
        if let Some(code) = rmap.get(n) {
            by_region.entry(code.clone()).or_default().push(n.clone());
        }
    }

    let mut out = Vec::new();
    let mut us_group_name = None;
    if !us_nodes.is_empty() {
        let base_name = "美国优选";
        let group_name = if node_names.iter().any(|n| n == base_name) {
            format!("{} (region)", base_name)
        } else {
            base_name.to_string()
        };
        us_group_name = Some(group_name.clone());
        out.push(make_url_test_group(&group_name, us_nodes));
    }

    let mut codes: Vec<&String> = by_region.keys().filter(|c| c.as_str() != "US").collect();
    codes.sort();
    for code in codes {
        let nodes = by_region.get(code).unwrap();
        if nodes.is_empty() {
            continue;
        }
        let base_name = format!("{}优选", region_display_name(code));
        let group_name = if node_names.iter().any(|n| n == &base_name) {
            format!("{} (region)", base_name)
        } else {
            base_name
        };
        out.push(make_url_test_group(&group_name, nodes));
    }
    (out, us_group_name)
}

fn make_url_test_group(name: &str, nodes: &[String]) -> serde_yaml::Value {
    let mut m = serde_yaml::Mapping::new();
    m.insert(
        serde_yaml::Value::from("name"),
        serde_yaml::Value::from(name),
    );
    m.insert(
        serde_yaml::Value::from("type"),
        serde_yaml::Value::from("url-test"),
    );
    m.insert(
        serde_yaml::Value::from("url"),
        serde_yaml::Value::from("https://cp.cloudflare.com/generate_204"),
    );
    m.insert(
        serde_yaml::Value::from("interval"),
        serde_yaml::Value::from(300),
    );
    m.insert(
        serde_yaml::Value::from("tolerance"),
        serde_yaml::Value::from(100),
    );
    m.insert(
        serde_yaml::Value::from("expected-status"),
        serde_yaml::Value::from(204),
    );
    m.insert(
        serde_yaml::Value::from("timeout"),
        serde_yaml::Value::from(5000),
    );
    m.insert(
        serde_yaml::Value::from("proxies"),
        serde_yaml::Value::Sequence(
            nodes
                .iter()
                .map(|n| serde_yaml::Value::from(n.clone()))
                .collect(),
        ),
    );
    serde_yaml::Value::Mapping(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_runtime_config_strips_app_only_keys() {
        let mut app = Config::default();
        app.general.profile = "MY_PROFILE".to_string();
        app.locale = "en-US".to_string();
        app.general.mixed_port = 7788;
        app.proxy.external_controller = "127.0.0.1:11111".to_string();

        let runtime = build_runtime_config(&app, None, None).unwrap();
        let map = runtime.as_mapping().unwrap();

        // 应用级键不得进入运行时
        assert!(map.get("profile").is_none(), "profile must be stripped");
        assert!(map.get("locale").is_none(), "locale must be stripped");
        assert!(map.get("advanced").is_none(), "advanced must be stripped");
        assert!(map.get("profiles").is_none(), "profiles must be stripped");

        // 受控键携带应用值
        assert_eq!(map.get("mixed-port").unwrap().as_u64(), Some(7788));
        assert_eq!(
            map.get("external-controller").unwrap().as_str(),
            Some("127.0.0.1:11111")
        );
        assert_eq!(map.get("mode").unwrap().as_str(), Some("rule"));

        // 内置规则 / 组 / 提供者兜底
        assert!(map.get("rules").is_some());
        assert!(map.get("proxy-groups").is_some());
        assert!(map.get("rule-providers").is_some());
    }

    #[test]
    fn build_runtime_config_merges_bare_proxy_list() {
        let app = Config::default();
        let profile = r#"
proxies:
  - name: Node1
    type: ss
    server: 1.2.3.4
    port: 8388
    cipher: aes-128-gcm
    password: pwd
  - name: Node2
    type: vmess
    server: 5.6.7.8
    port: 443
    uuid: abc
    alterId: 0
    cipher: auto
"#;

        let runtime = build_runtime_config(&app, Some(profile), None).unwrap();
        let map = runtime.as_mapping().unwrap();

        // proxies 保留
        assert_eq!(
            map.get("proxies").unwrap().as_sequence().map(|s| s.len()),
            Some(2)
        );
        // 内置规则/组存在
        assert!(map.get("proxy-groups").is_some());
        assert!(map.get("rules").is_some());
        // 订阅节点注入叶子组：人工优选只含真实节点（无 DIRECT），
        // 自动优选（url-test）也只含真实节点——DIRECT 不是代理节点，
        // 注入它会让 url-test 把直连当作零延迟最优节点永久霸占自动组。
        let groups = map.get("proxy-groups").unwrap().as_sequence().unwrap();
        let manual = groups
            .iter()
            .find(|g| g.get("name").and_then(|n| n.as_str()) == Some("人工优选"))
            .unwrap();
        let manual_names: Vec<&str> = manual
            .get("proxies")
            .unwrap()
            .as_sequence()
            .unwrap()
            .iter()
            .filter_map(|p| p.as_str())
            .collect();
        assert_eq!(manual_names, vec!["自动优选", "Node1", "Node2"]);
        let auto = groups
            .iter()
            .find(|g| g.get("name").and_then(|n| n.as_str()) == Some("自动优选"))
            .unwrap();
        let auto_names: Vec<&str> = auto
            .get("proxies")
            .unwrap()
            .as_sequence()
            .unwrap()
            .iter()
            .filter_map(|p| p.as_str())
            .collect();
        assert_eq!(auto_names, vec!["Node1", "Node2"]);
    }

    #[test]
    fn build_runtime_config_zero_nodes_drops_auto_group_keeps_manual_direct() {
        // mihomo v1.19.x 拒绝空 proxies 的代理组（"`use` or `proxies` missing"）。
        // 零节点约束：自动优选只能含真实节点信息——不得注入任何占位，
        // 因此整组移除，且其余组的引用列表同步剔除避免悬空引用；
        // 人工优选（select）补 DIRECT 兜底使配置校验通过。
        let app = Config::default();
        let runtime = build_runtime_config(&app, None, None).unwrap();
        let groups = runtime
            .as_mapping()
            .unwrap()
            .get("proxy-groups")
            .unwrap()
            .as_sequence()
            .unwrap();

        // 自动优选整组移除
        assert!(
            !groups
                .iter()
                .any(|g| g.get("name").and_then(|n| n.as_str()) == Some("自动优选")),
            "auto group must be dropped when no nodes"
        );

        for name in ["GLOBAL", "扶梯出行", "人工智能", "影音视听"] {
            let group = groups
                .iter()
                .find(|g| g.get("name").and_then(|n| n.as_str()) == Some(name))
                .unwrap();
            let names: Vec<&str> = group
                .get("proxies")
                .unwrap()
                .as_sequence()
                .unwrap()
                .iter()
                .filter_map(|p| p.as_str())
                .collect();
            assert!(
                !names.contains(&"自动优选"),
                "{} must not reference dropped auto group",
                name
            );
        }

        // 人工优选补 DIRECT 兜底
        let manual = groups
            .iter()
            .find(|g| g.get("name").and_then(|n| n.as_str()) == Some("人工优选"))
            .unwrap();
        let names: Vec<&str> = manual
            .get("proxies")
            .unwrap()
            .as_sequence()
            .unwrap()
            .iter()
            .filter_map(|p| p.as_str())
            .collect();
        assert_eq!(names, vec!["DIRECT"]);
    }

    /// 区域分组：有美国节点时人工优选=[美国优选, 美国节点...]，
    /// 美国优选组生成（url-test 仅美国节点），GLOBAL/扶梯出行追加区域组引用。
    #[test]
    fn build_runtime_config_region_us_nodes_build_manual_and_region_groups() {
        let app = Config::default();
        let profile = r#"
proxies:
  - name: US1
    type: ss
    server: 8.8.8.8
    port: 8388
    cipher: aes-128-gcm
    password: x
  - name: HK1
    type: ss
    server: 1.2.4.1
    port: 8388
    cipher: aes-128-gcm
    password: x
"#;
        let mut region = std::collections::HashMap::new();
        region.insert("US1".to_string(), "US".to_string());
        region.insert("HK1".to_string(), "HK".to_string());

        let runtime = build_runtime_config(&app, Some(profile), Some(&region)).unwrap();
        let map = runtime.as_mapping().unwrap();
        let groups = map.get("proxy-groups").unwrap().as_sequence().unwrap();

        let find = |name: &str| {
            groups
                .iter()
                .find(|g| g.get("name").and_then(|n| n.as_str()) == Some(name))
                .unwrap_or_else(|| panic!("group {} missing", name))
        };

        // 人工优选：[美国优选, US1]，默认选美国优选（自动最低延迟美国节点）
        let manual = find("人工优选");
        let manual_names: Vec<&str> = manual
            .get("proxies")
            .unwrap()
            .as_sequence()
            .unwrap()
            .iter()
            .filter_map(|p| p.as_str())
            .collect();
        assert_eq!(manual_names, vec!["美国优选", "US1"]);

        // 美国优选：url-test 仅美国节点
        let us_group = find("美国优选");
        assert_eq!(
            us_group.get("type").and_then(|t| t.as_str()),
            Some("url-test")
        );
        let us_names: Vec<&str> = us_group
            .get("proxies")
            .unwrap()
            .as_sequence()
            .unwrap()
            .iter()
            .filter_map(|p| p.as_str())
            .collect();
        assert_eq!(us_names, vec!["US1"]);

        // 香港优选：url-test 仅香港节点
        let hk_group = find("香港优选");
        let hk_names: Vec<&str> = hk_group
            .get("proxies")
            .unwrap()
            .as_sequence()
            .unwrap()
            .iter()
            .filter_map(|p| p.as_str())
            .collect();
        assert_eq!(hk_names, vec!["HK1"]);

        // 自动优选：全部节点（不变）
        let auto = find("自动优选");
        let auto_names: Vec<&str> = auto
            .get("proxies")
            .unwrap()
            .as_sequence()
            .unwrap()
            .iter()
            .filter_map(|p| p.as_str())
            .collect();
        assert_eq!(auto_names, vec!["US1", "HK1"]);

        // GLOBAL / 扶梯出行 追加区域组引用（尾部，不动默认选中）
        let global = find("GLOBAL");
        let global_names: Vec<&str> = global
            .get("proxies")
            .unwrap()
            .as_sequence()
            .unwrap()
            .iter()
            .filter_map(|p| p.as_str())
            .collect();
        assert!(global_names.contains(&"美国优选"));
        assert!(global_names.contains(&"香港优选"));
        assert_eq!(global_names.first(), Some(&"DIRECT"));

        let futi = find("扶梯出行");
        let futi_names: Vec<&str> = futi
            .get("proxies")
            .unwrap()
            .as_sequence()
            .unwrap()
            .iter()
            .filter_map(|p| p.as_str())
            .collect();
        assert_eq!(futi_names.first(), Some(&"人工优选"));
        assert!(futi_names.contains(&"美国优选"));
        assert!(futi_names.contains(&"香港优选"));
    }

    /// 区域分组：无美国节点但有其他区域时，人工优选降级为 [自动优选, 全部节点...]，
    /// 其他区域组正常生成。
    #[test]
    fn build_runtime_config_region_no_us_falls_back_to_auto_first() {
        let app = Config::default();
        let profile = r#"
proxies:
  - name: HK1
    type: ss
    server: 1.2.4.1
    port: 8388
    cipher: aes-128-gcm
    password: x
  - name: JP1
    type: ss
    server: 1.2.4.2
    port: 8388
    cipher: aes-128-gcm
    password: x
"#;
        let mut region = std::collections::HashMap::new();
        region.insert("HK1".to_string(), "HK".to_string());
        region.insert("JP1".to_string(), "JP".to_string());

        let runtime = build_runtime_config(&app, Some(profile), Some(&region)).unwrap();
        let map = runtime.as_mapping().unwrap();
        let groups = map.get("proxy-groups").unwrap().as_sequence().unwrap();

        let find = |name: &str| {
            groups
                .iter()
                .find(|g| g.get("name").and_then(|n| n.as_str()) == Some(name))
        };

        // 无美国节点 → 不生成美国优选组
        assert!(
            find("美国优选").is_none(),
            "US group must not exist without US nodes"
        );

        // 人工优选降级：[自动优选, HK1, JP1]
        let manual = find("人工优选").unwrap();
        let manual_names: Vec<&str> = manual
            .get("proxies")
            .unwrap()
            .as_sequence()
            .unwrap()
            .iter()
            .filter_map(|p| p.as_str())
            .collect();
        assert_eq!(manual_names, vec!["自动优选", "HK1", "JP1"]);

        // 其他区域组生成
        assert!(find("香港优选").is_some());
        assert!(find("日本优选").is_some());
    }

    /// 区域分组：region_map 为 None（旧 profile）→ 完全退化为当前行为，
    /// 人工优选=[自动优选, 全部节点]，无动态区域组。
    #[test]
    fn build_runtime_config_region_none_falls_back_to_flat_behavior() {
        let app = Config::default();
        let profile = r#"
proxies:
  - name: N1
    type: ss
    server: 1.2.3.4
    port: 8388
    cipher: aes-128-gcm
    password: x
"#;
        let runtime = build_runtime_config(&app, Some(profile), None).unwrap();
        let map = runtime.as_mapping().unwrap();
        let groups = map.get("proxy-groups").unwrap().as_sequence().unwrap();

        let names: Vec<&str> = groups
            .iter()
            .filter_map(|g| g.get("name").and_then(|n| n.as_str()))
            .collect();
        // 固定 6 组，无任何动态区域组
        assert_eq!(
            names,
            vec![
                "GLOBAL",
                "扶梯出行",
                "人工智能",
                "影音视听",
                "人工优选",
                "自动优选"
            ]
        );

        let manual = groups
            .iter()
            .find(|g| g.get("name").and_then(|n| n.as_str()) == Some("人工优选"))
            .unwrap();
        let manual_names: Vec<&str> = manual
            .get("proxies")
            .unwrap()
            .as_sequence()
            .unwrap()
            .iter()
            .filter_map(|p| p.as_str())
            .collect();
        assert_eq!(manual_names, vec!["自动优选", "N1"]);
    }

    /// 区域分组：节点名与区域组名冲突（如节点恰好叫"香港优选"）→ 组名加 (region) 后缀。
    #[test]
    fn build_runtime_config_region_group_name_collision_gets_suffix() {
        let app = Config::default();
        let profile = r#"
proxies:
  - name: 香港优选
    type: ss
    server: 1.2.4.1
    port: 8388
    cipher: aes-128-gcm
    password: x
"#;
        let mut region = std::collections::HashMap::new();
        region.insert("香港优选".to_string(), "HK".to_string());

        let runtime = build_runtime_config(&app, Some(profile), Some(&region)).unwrap();
        let map = runtime.as_mapping().unwrap();
        let groups = map.get("proxy-groups").unwrap().as_sequence().unwrap();
        let names: Vec<&str> = groups
            .iter()
            .filter_map(|g| g.get("name").and_then(|n| n.as_str()))
            .collect();
        assert!(
            names.contains(&"香港优选 (region)"),
            "collision must be suffixed: {:?}",
            names
        );
        // 节点名本身保持不变
        let node_names: Vec<&str> = map
            .get("proxies")
            .unwrap()
            .as_sequence()
            .unwrap()
            .iter()
            .filter_map(|p| p.get("name").and_then(|n| n.as_str()))
            .collect();
        assert_eq!(node_names, vec!["香港优选"]);
    }

    #[test]
    fn build_runtime_config_subscription_restores_full_group_structure() {
        // 有真实节点时自动优选必须恢复生成，且只含真实节点名。
        let app = Config::default();
        let profile = r#"
proxies:
  - name: Node1
    type: ss
    server: 1.2.3.4
    port: 8388
    cipher: aes-128-gcm
    password: pwd
"#;
        let runtime = build_runtime_config(&app, Some(profile), None).unwrap();
        let groups = runtime
            .as_mapping()
            .unwrap()
            .get("proxy-groups")
            .unwrap()
            .as_sequence()
            .unwrap();

        let auto = groups
            .iter()
            .find(|g| g.get("name").and_then(|n| n.as_str()) == Some("自动优选"))
            .expect("auto group must exist with subscription");
        let names: Vec<&str> = auto
            .get("proxies")
            .unwrap()
            .as_sequence()
            .unwrap()
            .iter()
            .filter_map(|p| p.as_str())
            .collect();
        assert_eq!(names, vec!["Node1"]);

        // GLOBAL 引用完整（含自动优选）
        let global = groups
            .iter()
            .find(|g| g.get("name").and_then(|n| n.as_str()) == Some("GLOBAL"))
            .unwrap();
        let global_names: Vec<&str> = global
            .get("proxies")
            .unwrap()
            .as_sequence()
            .unwrap()
            .iter()
            .filter_map(|p| p.as_str())
            .collect();
        assert_eq!(
            global_names,
            vec!["DIRECT", "REJECT", "人工优选", "自动优选"]
        );
    }

    #[test]
    fn build_runtime_config_always_injects_subscription_nodes_into_leaf_groups() {
        let app = Config::default();
        let expected_rules = app.rules.len();
        let profile = r#"
proxies:
  - name: Fast
    type: ss
    server: 1.1.1.1
    port: 8388
    cipher: aes-128-gcm
    password: x
proxy-groups:
  - name: "🚀 节点选择"
    type: select
    proxies: [Fast, DIRECT]
rules:
  - GEOIP,CN,DIRECT
  - MATCH,"🚀 节点选择"
"#;

        let runtime = build_runtime_config(&app, Some(profile), None).unwrap();
        let map = runtime.as_mapping().unwrap();

        // 应用固定结构不被订阅覆盖：内置 6 组（GLOBAL + 5）
        let groups = map.get("proxy-groups").unwrap().as_sequence().unwrap();
        assert_eq!(groups.len(), 6, "built-in 6 groups kept");
        let group_names: Vec<&str> = groups
            .iter()
            .filter_map(|g| g.get("name").and_then(|n| n.as_str()))
            .collect();
        assert_eq!(
            group_names,
            vec![
                "GLOBAL",
                "扶梯出行",
                "人工智能",
                "影音视听",
                "人工优选",
                "自动优选"
            ]
        );
        // 订阅节点强制注入叶子组（即使订阅自带 proxy-groups/rules）：
        // 人工优选与自动优选都只含真实节点——url-test 不得含 DIRECT
        let manual = groups
            .iter()
            .find(|g| g.get("name").and_then(|n| n.as_str()) == Some("人工优选"))
            .unwrap();
        let manual_names: Vec<&str> = manual
            .get("proxies")
            .unwrap()
            .as_sequence()
            .unwrap()
            .iter()
            .filter_map(|p| p.as_str())
            .collect();
        assert_eq!(manual_names, vec!["自动优选", "Fast"]);
        let auto = groups
            .iter()
            .find(|g| g.get("name").and_then(|n| n.as_str()) == Some("自动优选"))
            .unwrap();
        let auto_names: Vec<&str> = auto
            .get("proxies")
            .unwrap()
            .as_sequence()
            .unwrap()
            .iter()
            .filter_map(|p| p.as_str())
            .collect();
        assert_eq!(auto_names, vec!["Fast"]);
        // 内置规则保留（引用内置组），订阅自带规则不采用
        assert_eq!(
            map.get("rules").unwrap().as_sequence().map(|s| s.len()),
            Some(expected_rules)
        );
    }

    #[test]
    fn build_runtime_config_profile_cannot_override_app_settings() {
        let app = Config::default(); // mixed-port 7890
        let profile = r#"
mixed-port: 9999
mode: global
external-controller: 0.0.0.0:9999
external-ui: ./evil
dns:
  enable: false
tun:
  enable: true
listeners:
  - name: evil
hosts:
  example.com: 1.2.3.4
sniffer:
  enable: true
script:
  code: evil
proxy-providers:
  p:
    type: http
    url: https://evil.com/x.yaml
    path: "C:\\evil.yaml"
rule-providers:
  r:
    type: http
    url: https://evil.com/r.yaml
    path: "C:\\evil.yaml"
proxies:
  - name: Fast
    type: ss
    server: 1.1.1.1
    port: 8388
    cipher: aes-128-gcm
    password: x
"#;

        let runtime = build_runtime_config(&app, Some(profile), None).unwrap();
        let map = runtime.as_mapping().unwrap();

        // 受控键保持应用值
        assert_eq!(map.get("mixed-port").unwrap().as_u64(), Some(7890));
        assert_eq!(map.get("mode").unwrap().as_str(), Some("rule"));
        assert_eq!(
            map.get("external-controller").unwrap().as_str(),
            Some("127.0.0.1:9090")
        );
        assert_eq!(
            map.get("secret").unwrap().as_str(),
            Some("clash-edge-secret")
        );
        // 未知顶层字段不透传——订阅携带的 hosts/sniffer/script/listeners/
        // external-ui/dns/tun/proxy-providers/rule-providers 均不得进入运行时
        assert!(map.get("hosts").is_none(), "hosts must not pass through");
        assert!(
            map.get("sniffer").is_none(),
            "sniffer must not pass through"
        );
        assert!(map.get("script").is_none(), "script must not pass through");
        assert!(
            map.get("listeners").is_none(),
            "listeners must not pass through"
        );
        assert!(
            map.get("external-ui").is_none(),
            "external-ui must not pass through"
        );
        assert!(
            map.get("proxy-providers").is_none(),
            "proxy-providers must not pass through"
        );
        // dns/tun 保持应用值（受控键），不被订阅覆盖
        assert_eq!(
            map.get("dns")
                .unwrap()
                .get("enable")
                .and_then(|v| v.as_bool()),
            Some(true),
            "dns stays at app default"
        );
        assert_eq!(
            map.get("tun")
                .unwrap()
                .get("enable")
                .and_then(|v| v.as_bool()),
            Some(false),
            "tun stays at app default"
        );
        // rule-providers 仅应用内置 5 组，订阅 rule-providers 被忽略
        let rp = map.get("rule-providers").unwrap().as_mapping().unwrap();
        assert!(
            rp.get("r").is_none(),
            "subscription rule-providers must be ignored"
        );
        assert!(
            rp.get("direct").is_some(),
            "builtin rule-providers preserved"
        );
        // 订阅节点仍生效
        assert_eq!(
            map.get("proxies").unwrap().as_sequence().map(|s| s.len()),
            Some(1)
        );
    }

    /// 导入配置的节点（AppConfig.extra.proxies）必须注入内置叶子组：
    /// 若只写顶层 proxies、不注入叶子组，人工优选保持 [DIRECT]、
    /// 自动优选被零节点兜底删除，所有流量直连（"导入配置不起效"）。
    #[test]
    fn build_runtime_config_imported_extra_proxies_inject_into_leaf_groups() {
        let mut app = Config::default();
        let nodes: serde_yaml::Value = serde_yaml::from_str(
            r#"
proxies:
  - name: Imported
    type: ss
    server: 1.1.1.1
    port: 8388
    cipher: aes-128-gcm
    password: x
"#,
        )
        .unwrap();
        let proxies = nodes.get("proxies").unwrap().clone();
        app.extra.insert("proxies".into(), proxies);

        // 无激活 Profile（read_active_profile → None）的典型导入场景
        let runtime = build_runtime_config(&app, None, None).unwrap();
        assert_eq!(
            runtime
                .as_mapping()
                .unwrap()
                .get("proxies")
                .unwrap()
                .as_sequence()
                .map(|s| s.len()),
            Some(1),
            "imported proxies must reach runtime"
        );

        // 叶子组注入导入节点名，自动优选恢复生成
        let groups = runtime.as_mapping().unwrap().get("proxy-groups").unwrap();
        let manual = groups
            .get("人工优选")
            .or_else(|| {
                groups.as_sequence().and_then(|s| {
                    s.iter()
                        .find(|g| g.get("name").and_then(|n| n.as_str()) == Some("人工优选"))
                })
            })
            .unwrap();
        let names: Vec<&str> = manual
            .get("proxies")
            .unwrap()
            .as_sequence()
            .unwrap()
            .iter()
            .filter_map(|p| p.as_str())
            .collect();
        assert_eq!(names, vec!["自动优选", "Imported"]);
    }

    /// 激活 Profile 带空 `proxies:` 列表时不得遮蔽导入节点（Some([]) 曾使
    /// effective 节点来源判空失败，运行时拿零节点、全部直连）。
    #[test]
    fn build_runtime_config_empty_profile_proxies_fall_back_to_extra() {
        let mut app = Config::default();
        let nodes: serde_yaml::Value = serde_yaml::from_str(
            "proxies:\n  - name: Imported\n    type: ss\n    server: 1.1.1.1\n    port: 8388\n    cipher: aes-128-gcm\n    password: x\n",
        )
        .unwrap();
        app.extra
            .insert("proxies".into(), nodes.get("proxies").unwrap().clone());

        let runtime = build_runtime_config(&app, Some("proxies: []\n"), None).unwrap();
        let map = runtime.as_mapping().unwrap();
        assert_eq!(
            map.get("proxies").unwrap().as_sequence().map(|s| s.len()),
            Some(1),
            "empty profile proxies must not shadow imported nodes"
        );
        let groups = map.get("proxy-groups").unwrap().as_sequence().unwrap();
        assert!(
            groups
                .iter()
                .any(|g| g.get("name").and_then(|n| n.as_str()) == Some("自动优选")),
            "auto group must be restored when imported nodes exist"
        );
    }

    /// 激活 Profile 提供真实节点时仍优先于 extra.proxies（订阅为主来源）。
    #[test]
    fn build_runtime_config_profile_proxies_still_win_over_extra() {
        let mut app = Config::default();
        let nodes: serde_yaml::Value = serde_yaml::from_str(
            "proxies:\n  - name: Imported\n    type: ss\n    server: 1.1.1.1\n    port: 8388\n    cipher: aes-128-gcm\n    password: x\n",
        )
        .unwrap();
        app.extra
            .insert("proxies".into(), nodes.get("proxies").unwrap().clone());

        let profile = r#"
proxies:
  - name: FromProfile
    type: ss
    server: 2.2.2.2
    port: 8388
    cipher: aes-128-gcm
    password: y
"#;
        let runtime = build_runtime_config(&app, Some(profile), None).unwrap();
        let names: Vec<String> = runtime
            .as_mapping()
            .unwrap()
            .get("proxies")
            .unwrap()
            .as_sequence()
            .unwrap()
            .iter()
            .filter_map(|p| p.get("name").and_then(|n| n.as_str()).map(str::to_string))
            .collect();
        assert_eq!(names, vec!["FromProfile"]);
    }

    /// C7：sniffer 默认不写入；开启后写入内置嗅探配置；订阅的 sniffer 永不透传。
    #[test]
    fn sniffer_is_opt_in_and_never_from_subscription() {
        let mut app = Config::default();
        let profile = "sniffer:\n  enable: true\n  evil: 1\nproxies: []\n";
        let off = build_runtime_config(&app, Some(profile), None).unwrap();
        assert!(off.as_mapping().unwrap().get("sniffer").is_none());
        app.general.sniffer = true;
        let on = build_runtime_config(&app, Some(profile), None).unwrap();
        let sn = on.as_mapping().unwrap().get("sniffer").unwrap();
        assert_eq!(sn.get("enable").and_then(|v| v.as_bool()), Some(true));
        assert!(
            sn.get("evil").is_none(),
            "subscription sniffer must not leak in"
        );
        assert_eq!(
            sn.get("override-destination").and_then(|v| v.as_bool()),
            Some(false)
        );
    }

    /// C7：strict-route 随配置输出；未设置网卡名时不输出 `interface-name: null`。
    #[test]
    fn tun_strict_route_and_no_null_interface_name() {
        let mut app = Config::default();
        app.tun.strict_route = true;
        let rt = build_runtime_config(&app, None, None).unwrap();
        let tun = rt.as_mapping().unwrap().get("tun").unwrap();
        assert_eq!(
            tun.get("strict-route").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert!(tun.get("interface-name").is_none());
    }

    /// A4：旧配置里的内置 http provider 迁移为本地 file；自定义 provider 不动。
    #[test]
    fn merge_rules_migrates_builtin_http_rule_providers() {
        let mut app = Config::default();
        let old: serde_yaml::Value = serde_yaml::from_str(
            "type: http\nbehavior: classical\nurl: https://raw.githubusercontent.com/akaspyrean/external/main/rules/ai.yaml\npath: ./rules/ai.yaml\ninterval: 86400\n",
        )
        .unwrap();
        app.rule_providers.insert("ai".into(), old);
        let custom: serde_yaml::Value = serde_yaml::from_str(
            "type: http\nbehavior: domain\nurl: https://example.com/mine.yaml\npath: ./rules/mine.yaml\n",
        )
        .unwrap();
        app.rule_providers.insert("mine".into(), custom.clone());
        let merged = merge_rules(app);
        assert_eq!(merged.rule_providers["ai"]["type"].as_str(), Some("file"));
        assert_eq!(
            merged.rule_providers["ai"]["path"].as_str(),
            Some("./rules/ai.yaml")
        );
        assert_eq!(merged.rule_providers["mine"], custom);
        // 缺失的内置项被补齐
        for n in crate::config::model::BUILTIN_RULE_SETS {
            assert_eq!(
                merged.rule_providers[*n]["type"].as_str(),
                Some("file"),
                "{}",
                n
            );
        }
    }

    #[test]
    fn merge_rules_accepts_mihomo_values() {
        let mut app = Config::default();
        app.general.proxy_mode = "script".to_string();
        app.general.find_process_mode = "always".to_string();
        let merged = merge_rules(app);

        // script 非法 → rule；always 合法 → 保留
        assert_eq!(merged.general.proxy_mode, "rule");
        assert_eq!(merged.general.find_process_mode, "always");
    }

    /// 订阅携带的 `proxy-providers` / `rule-providers` 不透传到运行时配置
    /// （白名单仅允许 `proxies`）。运行时仅保留应用内置 5 组 rule-providers，
    /// 其合法相对路径保持原样。
    #[test]
    fn build_runtime_config_drops_subscription_providers() {
        let app = Config::default();
        let profile = r#"
proxy-providers:
  p1:
    type: http
    url: https://example.com/x.yaml
    path: "C:\\evil.yaml"
rule-providers:
  r1:
    type: http
    behavior: classical
    url: https://example.com/r.yaml
    path: "C:\\evil.yaml"
"#;

        let runtime = build_runtime_config(&app, Some(profile), None).unwrap();
        let map = runtime.as_mapping().unwrap();

        // proxy-providers 不透传
        assert!(
            map.get("proxy-providers").is_none(),
            "subscription proxy-providers must not pass through"
        );

        // rule-providers：仅应用内置 5 组，订阅 r1 被丢弃
        let rp = map.get("rule-providers").unwrap().as_mapping().unwrap();
        assert!(
            rp.get("r1").is_none(),
            "subscription rule-providers must be dropped"
        );
        // 内置 rule-providers 兜底不破坏：合法相对路径保持原样
        assert_eq!(
            rp["direct"]["path"].as_str(),
            Some("./rules/direct.yaml"),
            "builtin safe relative path must be preserved"
        );
        assert_eq!(
            rp["ai"]["path"].as_str(),
            Some("./rules/ai.yaml"),
            "builtin safe relative path must be preserved"
        );
    }

    /// G：非法 TUN stack（空 / 未知 / 旧值）在 merge_rules 归一为 mixed。
    #[test]
    fn merge_rules_normalizes_invalid_tun_stack() {
        for bad in ["", "unknown", "wintun", "mixed-old"] {
            let mut app = Config::default();
            app.tun.stack = bad.to_string();
            let merged = merge_rules(app);
            assert_eq!(
                merged.tun.stack, "mixed",
                "invalid stack {:?} must normalize to mixed",
                bad
            );
        }
    }

    /// 合法 TUN stack（mixed / system / gvisor）在 merge_rules 保持不变。
    #[test]
    fn merge_rules_preserves_valid_tun_stack() {
        for good in ["mixed", "system", "gvisor"] {
            let mut app = Config::default();
            app.tun.stack = good.to_string();
            let merged = merge_rules(app);
            assert_eq!(merged.tun.stack, good);
        }
    }

    /// E：build_runtime_config 输出完整 TUN 段——enable/stack/auto-route/
    /// auto-detect-interface/dns-hijack 全部正确序列化给 mihomo。
    #[test]
    fn build_runtime_config_emits_full_tun_section() {
        let mut app = Config::default();
        app.tun.enable = true;
        let runtime = build_runtime_config(&app, None, None).unwrap();
        let tun = runtime
            .as_mapping()
            .unwrap()
            .get("tun")
            .expect("runtime must carry tun")
            .as_mapping()
            .expect("tun must be a mapping");

        assert_eq!(
            tun.get("enable").and_then(|v| v.as_bool()),
            Some(true),
            "tun.enable"
        );
        assert_eq!(
            tun.get("stack").and_then(|v| v.as_str()),
            Some("mixed"),
            "tun.stack"
        );
        assert_eq!(
            tun.get("auto-route").and_then(|v| v.as_bool()),
            Some(true),
            "tun.auto-route"
        );
        assert_eq!(
            tun.get("auto-detect-interface").and_then(|v| v.as_bool()),
            Some(true),
            "tun.auto-detect-interface"
        );
        let hijack: Vec<&str> = tun
            .get("dns-hijack")
            .expect("tun must carry dns-hijack")
            .as_sequence()
            .map(|s| s.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();
        assert_eq!(hijack, vec!["any:53", "tcp://any:53"], "tun.dns-hijack");
    }

    /// F：mixed / system / gvisor 三种 stack 均能正确保存并生成 runtime 配置。
    #[test]
    fn build_runtime_config_roundtrips_all_tun_stacks() {
        for stack in ["mixed", "system", "gvisor"] {
            let mut app = Config::default();
            app.tun.stack = stack.to_string();
            let runtime = build_runtime_config(&app, None, None).unwrap();
            let tun = runtime
                .as_mapping()
                .unwrap()
                .get("tun")
                .unwrap()
                .as_mapping()
                .unwrap();
            assert_eq!(
                tun.get("stack").and_then(|v| v.as_str()),
                Some(stack),
                "stack {} must roundtrip into runtime",
                stack
            );
        }
    }
}
