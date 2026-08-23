// src-tauri/src/core/config.rs
//! 配置加载/保存 与 运行时配置（MihomoConfig）生成
//!
//! AppConfig（Data/config.yaml，应用单一数据源）与 MihomoConfig
//! （Data/runtime-config.yaml，mihomo 实际加载）分离：
//! - AppConfig 完整保留应用级字段与订阅未知键（`#[serde(flatten)]` 兜底）；
//! - 启动 / 重载时由 `build_runtime_config` 从「AppConfig + 激活 Profile 内容」
//!   生成只含 mihomo 顶层合法键的运行时配置，以 `-f` 交给 mihomo。

use tracing::{debug, warn};

use crate::config::model::Config;
use crate::util::error::{Error, Result};

/// mihomo 顶层 `mode` 合法值。
/// 官方 config.yaml 模板仅 rule / global / direct 三值；
/// `script` 是 Clash Premium 的遗留，mihomo 不接受。
const VALID_PROXY_MODES: &[&str] = &["rule", "global", "direct"];

/// mihomo `find-process-mode` 合法值。
/// 官方模板注释：`# find-process-mode has 3 values: always, strict, off`。
const VALID_FIND_PROCESS_MODES: &[&str] = &["off", "strict", "always"];

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

/// 合并配置规则 - profile-preprocessor.cjs 逻辑的 Rust 实现
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

    // 规则提供者：为空则使用默认订阅源（由订阅管理器填充）
    if config.rule_providers.is_empty() {
        debug!("No rule-providers configured, will use default subscription sources");
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
/// 合并策略（复刻 ClashEdge profile-preprocessor 的 preset 合并语义）：
/// 1. AppConfig 控制运行时关键设置（端口 / 控制器 / 模式 / TUN / DNS），订阅不得覆盖；
/// 2. AppConfig.extra 兜底键作为基线透传（用户自行导入的自定义键）；
/// 3. 激活 Profile 只提供节点：应用始终采用内置组骨架（GLOBAL + 5 组）与内置规则链，
///    订阅节点名强制注入叶子组（人工优选只含真实节点 / 自动优选含 DIRECT 兜底）；订阅自带的 proxy-groups/rules 不采用；
/// 4. rule-providers：AppConfig 内置 5 组为底，订阅同名覆盖；
/// 5. 其余订阅键（hosts / sniffer / proxy-providers / script 等）原样透传，
///    但 proxy-providers / rule-providers 的 `path` 统一强制限定在 `providers/` 下
///    （C3 规范化，防止订阅任意路径写盘）。
pub fn build_runtime_config(
    app: &Config,
    profile_content: Option<&str>,
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
    // P1-5：allow-lan 高级控制。仅 allow-lan=true 时写入：
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
            .filter(|s| {
                s.parse::<std::net::IpAddr>().is_ok()
                    || s.split_once('/')
                        .is_some_and(|(ip, _)| ip.trim().parse::<std::net::IpAddr>().is_ok())
            })
            .collect();
        if !allowed.is_empty() {
            put!("lan-allowed-ips", serde_yaml::to_value(&allowed)?);
        }
    }
    put!(
        "mode",
        serde_yaml::Value::from(app.general.proxy_mode.clone())
    );
    // 日志级别实装（P1-11）：设置页的 log-level 真正写入运行时配置，
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
    let profile_proxies = profile_map
        .get("proxies")
        .and_then(|v| v.as_sequence())
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
    //    应用始终采用内置组骨架（GLOBAL + 5 组）与内置规则链——这是应用的核心结构
    //    （规则模式固定 5 组）。订阅自带的 proxy-groups/rules 不采用（其规则引用的组
    //    在应用中不存在，整组采用会导致叶子组拿不到节点）。订阅只提供节点：
    //    节点名强制注入叶子组——人工优选（手动选择）只含真实节点不含 DIRECT，
    //    自动优选（url-test）只注入真实代理节点——DIRECT 不是代理节点，注入它会让
    //    url-test 把直连当作零延迟最优节点永久霸占自动组，所有真实节点拿不到流量。
    let mut groups = app.proxy_groups.clone();
    if let Some(proxies) = &profile_proxies {
        let node_names: Vec<String> = proxies
            .iter()
            .filter_map(|p| p.get("name").and_then(|n| n.as_str()).map(str::to_string))
            .collect();
        if !node_names.is_empty() {
            for group in groups.iter_mut() {
                let Some(gmap) = group.as_mapping_mut() else {
                    continue;
                };
                let Some(gname) = gmap.get("name").and_then(|n| n.as_str()) else {
                    continue;
                };
                let is_leaf = gname == "人工优选" || gname == "自动优选";
                if !is_leaf {
                    continue;
                }
                if let Some(plist) = gmap.get_mut("proxies").and_then(|p| p.as_sequence_mut()) {
                    plist.clear();
                    // 人工优选与自动优选都只注入真实代理节点。
                    // 自动优选（url-test）不得含 DIRECT：url-test 会把直连当
                    // 作零延迟节点永远选中，真实节点永远拿不到流量。
                    for n in &node_names {
                        plist.push(serde_yaml::Value::from(n.clone()));
                    }
                }
            }
        }
        put!("proxies", serde_yaml::to_value(proxies)?);
    } else if let Some(p) = app.extra.get("proxies") {
        put!("proxies", p.clone());
    }

    // 6) 零节点兜底：mihomo（v1.19.x）拒绝 proxies 为空的代理组
    //    （"`use` or `proxies` missing"，配置校验直接失败，核心无法启动）。
    //    约束：自动优选是 url-test 组，只能含真实节点——DIRECT/REJECT 占位
    //    都不行（url-test 会把 DIRECT 当零延迟节点永久霸占，REJECT 则黑掉流量）。
    //    故零节点时不生成自动优选组，并从其余组的 proxies 引用中同步剔除，
    //    保证不存在悬空引用；人工优选（select）补 DIRECT 兜底保持直连可用。
    //    一旦订阅提供节点，step 5 会注入真实节点名，自动优选恢复生成。
    let mut has_auto_group = false;
    for group in groups.iter_mut() {
        let Some(gmap) = group.as_mapping_mut() else {
            continue;
        };
        let Some(gname) = gmap.get("name").and_then(|n| n.as_str()) else {
            continue;
        };
        if gname == "自动优选" {
            let is_empty = gmap
                .get("proxies")
                .and_then(|p| p.as_sequence())
                .map(|s| s.is_empty())
                .unwrap_or(true);
            if is_empty {
                has_auto_group = true;
            }
        }
    }
    if has_auto_group {
        groups.retain(|g| g.get("name").and_then(|n| n.as_str()) != Some("自动优选"));
        for group in groups.iter_mut() {
            let Some(gmap) = group.as_mapping_mut() else {
                continue;
            };
            // 从引用列表剔除自动优选
            if let Some(plist) = gmap.get_mut("proxies").and_then(|p| p.as_sequence_mut()) {
                plist.retain(|v| v.as_str() != Some("自动优选"));
            }
            // 人工优选为空时补 DIRECT：MATCH 兜底规则会把全部流量引向
            // 扶梯出行 → 叶子组，DIRECT 占位让无订阅状态保持直连可用。
            let Some(gname) = gmap.get("name").and_then(|n| n.as_str()) else {
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

        let runtime = build_runtime_config(&app, None).unwrap();
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

        let runtime = build_runtime_config(&app, Some(profile)).unwrap();
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
        assert_eq!(manual_names, vec!["Node1", "Node2"]);
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
        let runtime = build_runtime_config(&app, None).unwrap();
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
        let runtime = build_runtime_config(&app, Some(profile)).unwrap();
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

        let runtime = build_runtime_config(&app, Some(profile)).unwrap();
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
        assert_eq!(manual_names, vec!["Fast"]);
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

        let runtime = build_runtime_config(&app, Some(profile)).unwrap();
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
        // P0-2：未知顶层字段不透传——订阅携带的 hosts/sniffer/script/listeners/
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

    /// P0-2：订阅携带的 `proxy-providers` / `rule-providers` 不再透传到运行时配置
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

        let runtime = build_runtime_config(&app, Some(profile)).unwrap();
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
}
