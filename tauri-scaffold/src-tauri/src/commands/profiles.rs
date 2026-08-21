// src-tauri/src/commands/profiles.rs
//! 配置文件命令：列表、新建、删除、重命名、激活、编辑、导入、导出
//!
//! 安全：所有 `profiles/<name>.yaml` 路径构造必须先过
//! `util::paths::sanitize_profile_name`（防路径穿越），
//! 否则 `name = "..\\..\\config.yaml"` 会越权读写任意文件。
//! 一致性：激活走统一编排层 `core::runtime::activate_profile`
//! （校验 → 持久化 → 重生成运行时配置 → 热重载核心 → 失败回滚），
//! 删除/重命名激活中的 Profile 时同步修正激活标记。

use tauri::{command, AppHandle, Manager};

use crate::util::error::{Error, Result};
use crate::util::paths::{get_profiles_dir, sanitize_profile_name};

// P1-8：订阅资源限制
/// 最大下载大小（10 MB）
const MAX_DOWNLOAD_BYTES: u64 = 10 * 1024 * 1024;
/// 最大 YAML 内容大小（10 MB 文本）
const MAX_YAML_CONTENT_BYTES: u64 = 10 * 1024 * 1024;
/// 最大节点数量
const MAX_NODE_COUNT: usize = 1000;
/// 最大节点名称长度
const MAX_NODE_NAME_LENGTH: usize = 100;
/// 任意字段值的最大长度（防止异常超长字段）
const MAX_FIELD_VALUE_LENGTH: usize = 5000;

/// 校验订阅内容是否符合资源限制（P1-8）。
/// 在写入磁盘前调用，防止恶意超大/超长订阅导致资源耗尽或 UI 异常。
fn validate_subscription_content(text: &str) -> Result<()> {
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
            // P1-9：按协议校验必要字段，不使用统一极小字段白名单。
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

/// P1-9：按协议校验代理节点必要字段。
/// 每种协议有各自的必需字段，统一校验可防止字段缺失导致 mihomo 启动失败。
/// 不使用统一的极小字段白名单——保证 VLESS/Reality/Trojan/Hysteria2/TUIC
/// 等现有协议能力不被破坏。
fn validate_node_protocol(node: &serde_yaml::Value, index: usize) -> Result<()> {
    let Some(map) = node.as_mapping() else {
        return Err(Error::Subscription(format!("Node #{} is not a mapping", index)));
    };
    let get_str = |key: &str| map.get(serde_yaml::Value::String(key.to_string()))
        .and_then(|v| v.as_str());
    let has = |key: &str| map.contains_key(serde_yaml::Value::String(key.to_string()));
    let protocol = get_str("type").unwrap_or("unknown");
    let name = get_str("name").unwrap_or("unnamed");
    // 通用字段：所有协议都需要 name 和 server
    if !has("server") {
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
            if !has("port") || !has("cipher") || !has("password") || !has("protocol") || !has("obfs") {
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
        "http" | "https" => {
            if !has("port") {
                return Err(Error::Subscription(format!(
                    "Node #{} ('{}', {}) requires port",
                    index, name, protocol
                )));
            }
        }
        "socks5" => {
            if !has("port") {
                return Err(Error::Subscription(format!(
                    "Node #{} ('{}', {}) requires port",
                    index, name, protocol
                )));
            }
        }
        _ => {
            // 未知协议：仅要求 name 和 server（已校验），不阻断导入
            // 以便支持未来 mihomo 新增协议
        }
    }
    Ok(())
}

/// 构造净化后的 profile 文件路径（所有 profile 命令统一入口）
fn profile_path(profiles_dir: &std::path::Path, name: &str) -> Result<std::path::PathBuf> {
    let safe = sanitize_profile_name(name)?;
    Ok(profiles_dir.join(format!("{}.yaml", safe)))
}

/// 当前激活的 profile 名（来自共享配置）
fn active_profile(app: &AppHandle) -> String {
    app.state::<crate::AppState>()
        .config_manager
        .lock()
        .unwrap()
        .get_config()
        .general
        .profile
}

#[command]
pub async fn list_profiles(app: AppHandle) -> Result<Vec<serde_json::Value>> {
    let profiles_dir = get_profiles_dir(&app)?;
    let active = active_profile(&app);
    let mut profiles = Vec::new();

    if profiles_dir.exists() {
        for entry in std::fs::read_dir(profiles_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("yaml") {
                let name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_string();
                profiles.push(serde_json::json!({
                    "name": name,
                    // 仅暴露文件名，不透传绝对路径（避免向 WebView 泄露数据目录结构）
                    "path": path
                        .file_name()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    "active": name == active,
                    // P1-10：订阅地址脱敏——前端只获得 host 或脱敏 URL，
                    // token/key 不返回前端。后端更新功能读回文件内完整 URL 重新拉取。
                    "url": std::fs::read_to_string(&path)
                        .ok()
                        .and_then(|c| extract_subscribe_url(&c))
                        .and_then(|u| redact_subscribe_url(&u)),
                }));
            }
        }
    }

    Ok(profiles)
}

#[command]
pub async fn create_profile(app: AppHandle, name: String, content: Option<String>) -> Result<()> {
    let profiles_dir = get_profiles_dir(&app)?;
    let file_path = profile_path(&profiles_dir, &name)?;

    if file_path.exists() {
        return Err(Error::InvalidArgument("Profile already exists".to_string()));
    }

    // 空内容走内置模板（带默认 DNS/端口等，且 proxies/groups/rules 为空时
    // build_runtime_config 会自动回退内置骨架，不会产生无法启动的空配置）。
    let yaml = content.unwrap_or_else(|| {
        r#"
mixed-port: 7890
allow-lan: false
mode: rule
log-level: info
ipv6: false
dns:
  enable: true
  listen: 127.0.0.1:9053
  ipv6: false
  enhanced-mode: fake-ip
  fake-ip-range: 198.18.0.1/16
  default-nameserver:
    - 223.5.5.5
    - 119.29.29.29
  nameserver:
    - https://dns.alidns.com/dns-query
    - https://doh.pub/dns-query
"#
        .trim()
        .to_string()
    });

    std::fs::write(&file_path, yaml)?;
    Ok(())
}

#[command]
pub async fn delete_profile(app: AppHandle, name: String) -> Result<()> {
    let profiles_dir = get_profiles_dir(&app)?;
    let file_path = profile_path(&profiles_dir, &name)?;

    if !file_path.exists() {
        return Err(Error::NotFound("Profile not found".to_string()));
    }

    std::fs::remove_file(file_path)?;

    // 若删除的是激活中的 Profile，激活标记不能指向已不存在的文件：
    // 重置回内置预设 DIRECT 并重载核心（失败仅记录，不阻塞删除）。
    let was_active = active_profile(&app) == name;
    if was_active {
        let _ = crate::core::runtime::activate_profile(&app, "DIRECT").await;
    }

    Ok(())
}

#[command]
pub async fn rename_profile(app: AppHandle, old_name: String, new_name: String) -> Result<()> {
    let profiles_dir = get_profiles_dir(&app)?;
    let old_path = profile_path(&profiles_dir, &old_name)?;
    let new_path = profile_path(&profiles_dir, &new_name)?;

    if !old_path.exists() {
        return Err(Error::NotFound("Profile not found".to_string()));
    }
    if new_path.exists() {
        return Err(Error::InvalidArgument("Profile already exists".to_string()));
    }

    std::fs::rename(&old_path, &new_path)?;

    // 重命名激活中的 Profile 时同步激活标记，避免界面显示已失效的激活名。
    if active_profile(&app) == old_name {
        let state = app.state::<crate::AppState>();
        {
            let mut cfg_mgr = state.config_manager.lock().unwrap();
            let mut cfg = cfg_mgr.get_config();
            cfg.general.profile = new_name.clone();
            cfg_mgr.set_config(cfg)?;
        }
        // 运行中的核心需要重载才能加载新文件名；失败回退原逻辑（重命名不因此失败）
        let core_guard = state.core_manager.lock().await;
        if let Some(core) = core_guard.as_ref() {
            let _ = core.reload_config().await;
        }
    }

    Ok(())
}

#[command]
pub async fn activate_profile(app: AppHandle, name: String) -> Result<()> {
    crate::core::runtime::activate_profile(&app, &name).await
}

#[command]
pub async fn get_profile_content(app: AppHandle, name: String) -> Result<String> {
    let profiles_dir = get_profiles_dir(&app)?;
    let file_path = profile_path(&profiles_dir, &name)?;

    if !file_path.exists() {
        return Err(Error::NotFound("Profile not found".to_string()));
    }

    let content = std::fs::read_to_string(file_path)?;
    Ok(content)
}

#[command]
pub async fn update_profile_content(app: AppHandle, name: String, content: String) -> Result<()> {
    let profiles_dir = get_profiles_dir(&app)?;
    let file_path = profile_path(&profiles_dir, &name)?;

    if !file_path.exists() {
        return Err(Error::NotFound("Profile not found".to_string()));
    }

    // 校验是合法 YAML
    serde_yaml::from_str::<serde_yaml::Value>(&content)?;

    std::fs::write(file_path, content)?;
    Ok(())
}

#[command]
pub async fn import_profile(app: AppHandle, name: String, content: String) -> Result<()> {
    let profiles_dir = get_profiles_dir(&app)?;
    let file_path = profile_path(&profiles_dir, &name)?;

    if file_path.exists() {
        return Err(Error::InvalidArgument("Profile already exists".to_string()));
    }

    // 校验是合法 YAML
    serde_yaml::from_str::<serde_yaml::Value>(&content)?;

    std::fs::write(file_path, content)?;
    Ok(())
}

#[command]
pub async fn export_profile(app: AppHandle, name: String) -> Result<String> {
    let profiles_dir = get_profiles_dir(&app)?;
    let file_path = profile_path(&profiles_dir, &name)?;

    if !file_path.exists() {
        return Err(Error::NotFound("Profile not found".to_string()));
    }

    let content = std::fs::read_to_string(file_path)?;
    Ok(content)
}

#[command]
pub async fn import_profile_from_url(app: AppHandle, name: String, url: String) -> Result<()> {
    let parsed = reqwest::Url::parse(&url)
        .map_err(|e| Error::InvalidArgument(format!("Invalid URL: {}", e)))?;
    match parsed.scheme() {
        "http" | "https" => {}
        _ => {
            return Err(Error::InvalidArgument(
                "URL scheme must be http or https".to_string(),
            ))
        }
    }

    // C2 SSRF 防护：parse+scheme 校验后再做禁段校验（localhost/.local/回环/私网等）
    crate::util::fetch::validate_url(&url).await?;

    // 从 URL 推导文件名：去 query/fragment，取最后一段非空路径（去尾部斜杠）。
    // 推导结果与用户提供的名字一样要过 sanitize。
    let name = {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            parsed
                .path()
                .trim_end_matches('/')
                .rsplit('/')
                .find(|s| !s.is_empty())
                .unwrap_or("subscription")
                .to_string()
        } else {
            trimmed.to_string()
        }
    };

    let profiles_dir = get_profiles_dir(&app)?;
    let file_path = profile_path(&profiles_dir, &name)?;

    if file_path.exists() {
        return Err(Error::InvalidArgument("Profile already exists".to_string()));
    }

    // 拉取动作直连优先：直连不通自动切应用自身代理兜底（软件代理模式不变）
    let resp = crate::util::fetch::get_direct_first(&app, &url).await?;

    if !resp.status().is_success() {
        return Err(Error::Other(format!(
            "Failed to fetch subscription: HTTP {}",
            resp.status()
        )));
    }

    let text = resp.text().await?;

    // P1-8：校验下载内容大小
    if text.len() as u64 > MAX_DOWNLOAD_BYTES {
        return Err(Error::Subscription(format!(
            "Download exceeds {} bytes limit",
            MAX_DOWNLOAD_BYTES
        )));
    }

    // P1-8：校验 YAML 合法 + 资源限制（节点数量/名称长度/字段值长度）
    validate_subscription_content(&text)?;

    // 顶部写入订阅地址注释头，供「更新」命令读回 URL 重新拉取。
    // 注释不影响 YAML 解析；订阅源自带的首行注释会原样保留在其正文中。
    // C6：写入用规范化后的 parsed.as_str()，而非用户原始字符串——
    // 原始串可能带换行/控制符/反斜杠注入到 YAML 注释头（反射注入）。
    std::fs::write(file_path, format!("# subscribe-url: {}\n{}\n", parsed.as_str(), text))?;
    Ok(())
}

/// 从 profile 文件内容提取订阅 URL（`# subscribe-url: <url>` 注释头）。
/// 无订阅地址的本地配置返回 None，前端据此不显示「更新」按钮。
fn extract_subscribe_url(content: &str) -> Option<String> {
    content.lines().find_map(|line| {
        let line = line.trim();
        let rest = line
            .strip_prefix("# subscribe-url:")
            .or_else(|| line.strip_prefix("#subscribe-url:"))?;
        let url = rest.trim();
        if url.is_empty() {
            None
        } else {
            Some(url.to_string())
        }
    })
}

/// P1-10：脱敏订阅 URL，返回 host 或脱敏 URL，不泄露 token/key。
///
/// - `https://user:pass@host:port/path?token=secret#frag` → `https://host:port/path`
/// - `https://host/path?token=secret` → `https://host/path`
/// - `https://host:port/path` → 原样返回（无敏感字段）
/// - 解析失败 → 返回 `"***"`（不泄露原始字符串）
fn redact_subscribe_url(url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(url).ok()?;
    let mut redacted = String::new();
    redacted.push_str(parsed.scheme());
    redacted.push_str("://");
    if let Some(host) = parsed.host_str() {
        redacted.push_str(host);
    }
    if let Some(port) = parsed.port() {
        if parsed.host_str().is_some() {
            redacted.push(':');
            redacted.push_str(&port.to_string());
        }
    }
    if !parsed.path().is_empty() && parsed.path() != "/" {
        redacted.push_str(parsed.path());
    }
    // 不包含 query（token/key 常在 query 参数里）和 fragment
    Some(redacted)
}

/// 更新订阅：读回 profile 顶部的订阅 URL，重新拉取内容并覆盖文件。
/// 若更新的是激活中的 Profile，走统一编排层 `activate_profile`
/// 重生成运行时配置并热重载核心，让新节点/规则立即生效（失败回滚）。
#[command]
pub async fn update_profile_subscription(app: AppHandle, name: String) -> Result<()> {
    let profiles_dir = get_profiles_dir(&app)?;
    let file_path = profile_path(&profiles_dir, &name)?;

    if !file_path.exists() {
        return Err(Error::NotFound("Profile not found".to_string()));
    }

    let content = std::fs::read_to_string(&file_path)?;
    let url = extract_subscribe_url(&content)
        .ok_or_else(|| Error::NotFound("Profile has no subscription URL".to_string()))?;

    // 校验 scheme（与导入一致）
    let parsed = reqwest::Url::parse(&url)
        .map_err(|e| Error::InvalidArgument(format!("Invalid URL: {}", e)))?;
    match parsed.scheme() {
        "http" | "https" => {}
        _ => {
            return Err(Error::InvalidArgument(
                "URL scheme must be http or https".to_string(),
            ))
        }
    }

    // C2 SSRF 防护：parse+scheme 校验后再做禁段校验
    crate::util::fetch::validate_url(&url).await?;

    // 拉取动作直连优先：直连不通自动切应用自身代理兜底（软件代理模式不变）
    let resp = crate::util::fetch::get_direct_first(&app, &url).await?;

    if !resp.status().is_success() {
        return Err(Error::Other(format!(
            "Failed to fetch subscription: HTTP {}",
            resp.status()
        )));
    }

    let text = resp.text().await?;

    // P1-8：校验下载内容大小
    if text.len() as u64 > MAX_DOWNLOAD_BYTES {
        return Err(Error::Subscription(format!(
            "Download exceeds {} bytes limit",
            MAX_DOWNLOAD_BYTES
        )));
    }

    // P1-8：校验资源限制（节点数量/名称长度/字段值长度）
    validate_subscription_content(&text)?;

    // 保留订阅地址注释头，供下次更新；正文为订阅源最新内容。
    // C6：写入用规范化后的 parsed.as_str()，防止原始字符串反射注入。
    std::fs::write(&file_path, format!("# subscribe-url: {}\n{}\n", parsed.as_str(), text))?;

    // 更新激活中的 Profile：热重载使新节点/规则生效
    if active_profile(&app) == name {
        crate::core::runtime::activate_profile(&app, &name).await?;
    }

    Ok(())
}
