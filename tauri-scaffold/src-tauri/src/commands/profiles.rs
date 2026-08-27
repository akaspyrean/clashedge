// src-tauri/src/commands/profiles.rs
//! 配置文件命令：列表、新建、删除、重命名、激活、编辑、导入、导出
//!
//! 安全：所有 `profiles/<name>.yaml` 路径构造必须先过
//! `util::paths::sanitize_profile_name`（防路径穿越），
//! 否则 `name = "..\\..\\config.yaml"` 会越权读写任意文件。
//! 一致性：激活走统一编排层 `core::runtime::activate_profile`
//! （校验 → 持久化 → 重生成运行时配置 → 热重载核心 → 失败回滚），
//! 删除/重命名激活中的 Profile 时同步修正激活标记。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use tauri::{command, AppHandle, Manager};
use tracing::{info, warn};

use crate::util::atomic::atomic_write;
use crate::util::error::{Error, Result};
use crate::util::paths::{get_profiles_dir, sanitize_profile_name};

/// 临时文件名计数器（与进程 id 组合成随机后缀，保证同一进程内不撞名）
static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

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

/// 构造净化后的 profile 文件路径（所有 profile 命令统一入口）
fn profile_path(profiles_dir: &std::path::Path, name: &str) -> Result<std::path::PathBuf> {
    let safe = sanitize_profile_name(name)?;
    Ok(profiles_dir.join(format!("{}.yaml", safe)))
}

/// 生成临时文件路径：`{path}.tmp.{pid}-{n}`（随机后缀，与目标同目录同文件系统）。
fn temp_path_for(path: &Path) -> PathBuf {
    let n = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".tmp.{}-{}", std::process::id(), n));
    path.with_file_name(name)
}

/// 生成备份文件路径：`{name}.yaml` -> `{name}.yaml.bak`
fn backup_path_for(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".bak");
    path.with_file_name(name)
}

/// 删除暂存路径：`{name}.yaml` -> `{name}.yaml.pending-delete`。
/// 扩展名不再是 `.yaml`，不会被 `list_profiles` 扫描到，删除中途失败时文件可恢复。
fn pending_delete_path_for(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".pending-delete");
    path.with_file_name(name)
}

/// 订阅 URL 脱敏：统一委托 `util::fetch::redact_url_for_log`（唯一实现）。
/// 仅保留 `scheme://host[:port]`，删除 userinfo/query/fragment 与完整 path
/// （path 可能携带 token，如 `/api/v1/client/<token>`）。解析失败返回 `"***"`。
fn redact_url(url: &str) -> String {
    crate::util::fetch::redact_url_for_log(url)
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

    // 非空内容需通过统一校验（与网络导入同一标准：大小/节点数/字段长度/协议）
    if let Some(ref c) = content {
        validate_subscription_content(c)?;
    }

    // 空内容走内置模板。产品架构里 Profile 只提供节点（runtime 仅透传 proxies），
    // 因此空模板就是空节点集，而非一份"看起来像完整 Mihomo 配置、实际大部分字段
    // 不生效"的误导性模板。
    let yaml = content.unwrap_or_else(|| "proxies: []\n".to_string());

    atomic_write(&file_path, yaml.as_bytes())?;
    Ok(())
}

#[command]
pub async fn delete_profile(app: AppHandle, name: String) -> Result<()> {
    let profiles_dir = get_profiles_dir(&app)?;
    let file_path = profile_path(&profiles_dir, &name)?;

    if !file_path.exists() {
        return Err(Error::NotFound("Profile not found".to_string()));
    }

    let was_active = active_profile(&app) == name;
    let pending = pending_delete_path_for(&file_path);

    // 事务化删除：先把正式文件暂存为 `.pending-delete`，切换激活成功后才真正删除。
    // 若切换激活失败，恢复暂存文件，保证原 profile 不丢失（避免"文件已删但激活态
    // 仍指向它"的不一致）。
    std::fs::rename(&file_path, &pending)?;

    // 删除的是激活中的 Profile：先重置回内置预设 DIRECT 并重载核心；失败恢复文件。
    if was_active {
        if let Err(e) = crate::core::runtime::activate_profile(&app, "DIRECT").await {
            let _ = std::fs::rename(&pending, &file_path);
            return Err(Error::Other(format!(
                "激活状态重置失败，已恢复原 Profile：{}",
                e
            )));
        }
    }

    // 提交：真正删除暂存文件。清理失败不视为删除失败（激活已切换、暂存文件不可见），
    // 仅告警，避免用户看到"删除失败"却已生效的矛盾反馈。
    if let Err(e) = std::fs::remove_file(&pending) {
        warn!("Profile deleted but pending file cleanup failed: {}", e);
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

    // 非激活 profile：仅重命名文件即可。
    if active_profile(&app) != old_name {
        std::fs::rename(&old_path, &new_path)?;
        return Ok(());
    }

    // 激活中的 profile：事务化重命名。
    //  1) rename 文件 old → new
    //  2) activate_profile(new)：统一持久化 config.profile=new + 重生成运行时 + 重启核心
    //     （成功：config=file=core 三者一致，并已刷新托盘/推送事件）
    //  3) 失败：activate_profile 内部已把 config.profile 回滚到旧名，这里再把
    //     文件 new → old 恢复，并重新 activate_profile(old) 让核心加载回旧文件。
    std::fs::rename(&old_path, &new_path)?;
    if let Err(e) = crate::core::runtime::activate_profile(&app, &new_name).await {
        // 回滚：config.profile 已被 activate_profile 回滚到旧名；这里恢复文件。
        let restored = std::fs::rename(&new_path, &old_path).is_ok();
        if let Err(e2) = crate::core::runtime::activate_profile(&app, &old_name).await {
            warn!("Rename rollback: re-activate '{}' failed: {}", old_name, e2);
        }
        if !restored {
            return Err(Error::Other(format!(
                "重命名激活失败，且恢复原文件也失败（'{}' 仍位于 '{}'）：{}",
                old_name, new_name, e
            )));
        }
        return Err(Error::Other(format!("重命名激活失败，已回滚：{}", e)));
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

    // 统一校验（与网络导入同一标准）
    validate_subscription_content(&content)?;

    // 事务式写：旧文件 → .bak，新内容 → target；激活中的 Profile 保存后立即生效，
    // 激活失败回滚 .bak 旧内容（避免"保存成功但运行还是旧节点"的界面≠运行不一致）。
    let temp_path = temp_path_for(&file_path);
    std::fs::write(&temp_path, content.as_bytes())?;
    commit_profile_file(&temp_path, &file_path)?;

    if active_profile(&app) == name {
        activate_with_rollback(&app, &name, &file_path).await?;
    }

    Ok(())
}

#[command]
pub async fn import_profile(app: AppHandle, name: String, content: String) -> Result<()> {
    let profiles_dir = get_profiles_dir(&app)?;
    let file_path = profile_path(&profiles_dir, &name)?;

    if file_path.exists() {
        return Err(Error::InvalidArgument("Profile already exists".to_string()));
    }

    // 统一校验（与网络导入同一标准）
    validate_subscription_content(&content)?;

    // 归一化为 proxies-only 节点集（兼容 proxy-providers 型订阅）
    let (normalized, _warnings) = normalize_subscription_body(&app, &content).await?;
    let normalized = format!("# profile: {}\n{}", name, normalized);
    validate_subscription_content(&normalized)?;

    atomic_write(&file_path, normalized.as_bytes())?;
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

    info!("Importing subscription from {}", redact_url(&url));

    // 流式下载到临时文件（Content-Length 预检 + chunk 累计上限），
    // 注释头先行写入；失败自动清理临时文件，不残留半成品。
    let temp_path = temp_path_for(&file_path);
    let header = format!("# subscribe-url: {}\n", parsed.as_str());
    download_subscription_streaming(&app, &url, &header, &temp_path).await?;

    // 校验 YAML 合法 + 资源限制（节点数量/名称长度/字段值长度）；失败清理临时文件
    let text = match std::fs::read_to_string(&temp_path) {
        Ok(t) => t,
        Err(e) => {
            let _ = std::fs::remove_file(&temp_path);
            return Err(e.into());
        }
    };
    if let Err(e) = validate_subscription_content(&text) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(e);
    }

    // 归一化为 proxies-only 节点集：兼容 proxy-providers 型订阅（Issue #1）。
    // 只保留节点，订阅自带的 groups/rules/hosts 等不进入存储（应用掌握策略结构）。
    let body = strip_subscribe_header(&text);
    let (normalized, warnings) = normalize_subscription_body(&app, &body).await?;
    if !warnings.is_empty() {
        warn!("Import '{}': {}", redact_url(&url), warnings.join("；"));
    }
    let final_text = format!("# subscribe-url: {}\n{}", parsed.as_str(), normalized);
    if let Err(e) = validate_subscription_content(&final_text) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(e);
    }
    // 归一化结果覆写临时文件后，再原子提交为正式文件
    std::fs::write(&temp_path, final_text.as_bytes())?;

    // 新文件：临时文件原子 rename 为正式文件（目标不存在，无需备份）
    commit_profile_file(&temp_path, &file_path)?;
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

/// 去掉 `# subscribe-url:` 注释头，返回订阅正文（供归一化解析）。
fn strip_subscribe_header(content: &str) -> String {
    content
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            !(t.starts_with("# subscribe-url:") || t.starts_with("#subscribe-url:"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 归一化订阅正文为 proxies-only 的 YAML 文档（Subscription Normalizer）。
/// 兼容仅含 `proxy-providers` 的现代订阅；返回 (归一化文档, 过程提示)。
async fn normalize_subscription_body(app: &AppHandle, body: &str) -> Result<(String, Vec<String>)> {
    let norm = crate::util::normalizer::normalize_subscription(app, body).await?;
    for w in &norm.warnings {
        warn!("Subscription normalization: {}", w);
    }
    let mut m = serde_yaml::Mapping::new();
    m.insert(
        serde_yaml::Value::String("proxies".into()),
        serde_yaml::Value::Sequence(norm.proxies),
    );
    let doc = serde_yaml::to_string(&serde_yaml::Value::Mapping(m))?;
    Ok((doc, norm.warnings))
}

/// P1-10：脱敏订阅 URL，返回 host（含可选端口），不泄露 token/key。
///
/// - `https://user:pass@host:port/path?token=secret#frag` → `https://host:port/…`
/// - `https://host/path?token=secret` → `https://host/…`
/// - `https://host:port` → `https://host:port`
/// - 解析失败 → 返回 None（不泄露原始字符串）
fn redact_subscribe_url(url: &str) -> Option<String> {
    reqwest::Url::parse(url).ok()?;
    Some(redact_url(url))
}

// P1 审计修复：流式大小限制 + 事务式替换
//
/// 流式下载订阅内容到临时文件（带 10MB 上限）。
/// - 响应头 Content-Length 超限：立即拒绝（不读 body）；
/// - 否则 `chunk()` 循环累计写入临时文件，累计超限即中止、删除临时文件并报错，
///   避免 `resp.text().await` 把恶意超大响应先整个读进内存。
/// - `header` 先行写入临时文件（订阅地址注释头，供「更新」命令读回 URL）；
///   其字节数计入总量上限。
async fn download_subscription_streaming(
    app: &AppHandle,
    url: &str,
    header: &str,
    temp_path: &Path,
) -> Result<()> {
    // 拉取动作直连优先：直连不通自动切应用自身代理兜底（软件代理模式不变）
    let mut resp = crate::util::fetch::get_direct_first(app, url).await?;

    if !resp.status().is_success() {
        return Err(Error::Other(format!(
            "Failed to fetch subscription: HTTP {}",
            resp.status()
        )));
    }

    if let Some(len) = resp.content_length() {
        if len > MAX_DOWNLOAD_BYTES {
            return Err(Error::Subscription(format!(
                "Download exceeds {} bytes limit",
                MAX_DOWNLOAD_BYTES
            )));
        }
    }

    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .create_new(true)
        .open(temp_path)?;
    let mut total: u64 = 0;
    file.write_all(header.as_bytes())?;
    total += header.len() as u64;
    while let Some(chunk) = resp.chunk().await? {
        total += chunk.len() as u64;
        if total > MAX_DOWNLOAD_BYTES {
            drop(file);
            let _ = std::fs::remove_file(temp_path);
            return Err(Error::Subscription(format!(
                "Download exceeds {} bytes limit",
                MAX_DOWNLOAD_BYTES
            )));
        }
        file.write_all(&chunk)?;
    }
    file.flush()?;
    Ok(())
}

/// 事务式用临时文件替换正式 profile 文件：
/// 1. 若旧文件存在，先重命名为 `{name}.yaml.bak`（残留旧备份先清理）；
/// 2. 临时文件 rename 为正式文件；
/// 3. 任一步失败：把 .bak 恢复原位、清理临时文件，返回 Err——
///    保证原来能工作的订阅不会因半途失败而丢失。
fn commit_profile_file(temp_path: &Path, target: &Path) -> Result<()> {
    let backup = backup_path_for(target);

    if target.exists() {
        let _ = std::fs::remove_file(&backup);
        std::fs::rename(target, &backup)?;
    }

    if let Err(e) = std::fs::rename(temp_path, target) {
        if backup.exists() && !target.exists() {
            let _ = std::fs::rename(&backup, target);
        }
        let _ = std::fs::remove_file(temp_path);
        return Err(e.into());
    }
    Ok(())
}

/// 激活 profile 并使其生效；激活失败时回滚已替换的文件（用 `.bak` 恢复旧内容）
/// 并重新激活旧内容，保证「文件 + config + 运行核心」三者一致。
///
/// 前置：调用方已通过 `commit_profile_file` 把旧文件备份到 `.bak`、新内容写到
/// target。若激活（重启核心加载新内容）失败，target 上是坏的新版本，`.bak` 里
/// 是仍能工作的旧版本——这里恢复旧版本并重新激活，避免"磁盘已是坏新版本、运行
/// 仍是旧状态"的半套状态。
async fn activate_with_rollback(app: &AppHandle, name: &str, file_path: &Path) -> Result<()> {
    if let Err(e) = crate::core::runtime::activate_profile(app, name).await {
        // 回滚：用 .bak 恢复旧内容。文件操作必须确认真的成功——若恢复也失败，
        // 要如实上报"操作失败且自动恢复失败"，并保留 backup 供手工恢复，而不是
        // 谎称"已回滚"。rename 在 Windows 上可替换已存在目标；失败则先删再 rename。
        let backup = backup_path_for(file_path);
        let restore_ok = std::fs::rename(&backup, file_path)
            .or_else(|_| {
                let _ = std::fs::remove_file(file_path);
                std::fs::rename(&backup, file_path)
            })
            .is_ok();

        if restore_ok {
            if let Err(e2) = crate::core::runtime::activate_profile(app, name).await {
                warn!("Rollback re-activate failed for '{}': {}", name, e2);
            }
            return Err(Error::Other(format!(
                "Profile '{}' 保存生效失败，已回滚到旧内容：{}",
                name, e
            )));
        }
        // 文件恢复失败：保留 backup，明确提示备份位置，绝不静默吞掉。
        warn!(
            "Profile '{}' rollback file restore FAILED; backup preserved at {}",
            name,
            backup.display()
        );
        return Err(Error::Other(format!(
            "Profile '{}' 保存生效失败，且自动恢复文件也失败（备份保留在 {}）：{}",
            name,
            backup.display(),
            e
        )));
    }
    Ok(())
}

/// 更新订阅：读回 profile 顶部的订阅 URL，重新拉取内容并覆盖文件。
/// 若更新的是激活中的 Profile，走统一编排层 `activate_profile`
/// 重生成运行时配置并热重载核心，让新节点/规则立即生效（失败回滚）。
#[command]
pub async fn update_profile_subscription(app: AppHandle, name: String) -> Result<()> {
    refresh_subscription(&app, &name).await
}

/// 订阅刷新核心逻辑（供命令与启动时静默刷新共用）：重新拉取订阅内容
/// 并事务式覆盖 profile 文件；激活中的 Profile 随后热重载生效。
pub async fn refresh_subscription(app: &AppHandle, name: &str) -> Result<()> {
    let profiles_dir = get_profiles_dir(app)?;
    let file_path = profile_path(&profiles_dir, name)?;

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

    info!("Updating subscription from {}", redact_url(&url));

    // 事务第一步：下载新内容到临时文件（流式 + 大小上限）。
    // 注释头先行写入，保留订阅地址供下次更新；
    // C6：URL 用规范化后的 parsed.as_str()，防止原始字符串反射注入。
    // 此阶段任何失败都不触碰现有文件，原订阅保持可用。
    let temp_path = temp_path_for(&file_path);
    let header = format!("# subscribe-url: {}\n", parsed.as_str());
    download_subscription_streaming(app, &url, &header, &temp_path).await?;

    // 内容校验在替换前完成；失败则清理临时文件并返回 Err
    let text = match std::fs::read_to_string(&temp_path) {
        Ok(t) => t,
        Err(e) => {
            let _ = std::fs::remove_file(&temp_path);
            return Err(e.into());
        }
    };
    if let Err(e) = validate_subscription_content(&text) {
        let _ = std::fs::remove_file(&temp_path);
        warn!(
            "Subscription update rejected for {}: {}",
            redact_url(&url),
            e
        );
        return Err(e);
    }

    // 归一化为 proxies-only 节点集（与导入一致，兼容 proxy-providers 型订阅）
    let body = strip_subscribe_header(&text);
    let (normalized, warnings) = normalize_subscription_body(app, &body).await?;
    if !warnings.is_empty() {
        warn!("Update '{}': {}", redact_url(&url), warnings.join("；"));
    }
    let final_text = format!("# subscribe-url: {}\n{}", parsed.as_str(), normalized);
    if let Err(e) = validate_subscription_content(&final_text) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(e);
    }
    std::fs::write(&temp_path, final_text.as_bytes())?;

    // 事务第二步/第三步：备份旧文件为 .bak，再把临时文件 rename 为正式文件；
    // 任一步失败自动恢复 .bak 并清理临时文件。
    commit_profile_file(&temp_path, &file_path)?;

    // 更新激活中的 Profile：热重载使新节点/规则生效；激活失败回滚 .bak 旧内容。
    if active_profile(app) == name {
        activate_with_rollback(app, name, &file_path).await?;
    }

    info!("Subscription updated: {}", redact_url(&url));
    Ok(())
}

// D6：启动时一次性订阅静默刷新
//
/// 订阅静默刷新的过期阈值：mtime 距今超过 24h 才刷新
const SUBSCRIPTION_REFRESH_AGE: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// 纯函数：从候选列表筛选需要静默刷新的 profile 名单（可测的阈值逻辑）。
///
/// - 不含 `# subscribe-url:` 头（非订阅）→ 跳过；
/// - mtime 未知 → 跳过（调用方负责对读取失败的场景 warn，避免误刷）；
/// - mtime 距 `now` 超过 `SUBSCRIPTION_REFRESH_AGE` → 刷新。
fn select_stale_subscriptions(
    candidates: Vec<(String, bool, Option<std::time::SystemTime>)>,
    now: std::time::SystemTime,
) -> Vec<String> {
    candidates
        .into_iter()
        .filter(|(_, has_url, mtime)| {
            *has_url
                && mtime
                    .map(|t| now.duration_since(t).unwrap_or_default() > SUBSCRIPTION_REFRESH_AGE)
                    .unwrap_or(false)
        })
        .map(|(name, _, _)| name)
        .collect()
}

/// 启动时一次性静默刷新过期订阅：遍历 profiles 目录，mtime 距今超过 24h
/// 且含订阅头的 .yaml 逐个串行刷新；单个失败仅 warn 不中断其他。
/// 无常驻定时器/循环——本函数执行完即返回，由调用方在启动流程末尾
/// 延迟触发一次。
pub async fn auto_refresh_stale_subscriptions(app: &AppHandle) {
    let profiles_dir = match get_profiles_dir(app) {
        Ok(dir) => dir,
        Err(e) => {
            warn!(
                "Auto subscription refresh skipped: cannot resolve profiles dir: {}",
                e
            );
            return;
        }
    };
    if !profiles_dir.exists() {
        return;
    }

    let mut candidates: Vec<(String, bool, Option<std::time::SystemTime>)> = Vec::new();
    let entries = match std::fs::read_dir(&profiles_dir) {
        Ok(entries) => entries,
        Err(e) => {
            warn!(
                "Auto subscription refresh skipped: cannot read {:?}: {}",
                profiles_dir, e
            );
            return;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("yaml") {
            continue;
        }
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        if name.is_empty() {
            continue;
        }
        // mtime 读取失败：跳过并 warn（保守处理，避免误刷）
        let mtime = match entry.metadata().and_then(|m| m.modified()) {
            Ok(t) => Some(t),
            Err(e) => {
                warn!(
                    "Auto subscription refresh: skip '{}' (mtime unavailable: {})",
                    name, e
                );
                None
            }
        };
        let has_url = std::fs::read_to_string(&path)
            .map(|c| extract_subscribe_url(&c).is_some())
            .unwrap_or(false);
        candidates.push((name, has_url, mtime));
    }

    let stale = select_stale_subscriptions(candidates, std::time::SystemTime::now());
    for name in stale {
        info!("Auto refreshing stale subscription: {}", name);
        if let Err(e) = refresh_subscription(app, &name).await {
            warn!("Auto subscription refresh failed for '{}': {}", name, e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(std::path::PathBuf);
    impl TempDir {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "clashedge-profiles-test-{}-{}-{}",
                tag,
                std::process::id(),
                TMP_COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }
        fn path(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn redact_url_drops_query_and_userinfo_and_path() {
        // path 也可能携带 token（如 /api/v1/client/<token>），必须一并丢弃。
        assert_eq!(
            redact_url("https://example.com/path?token=secret&id=1"),
            "https://example.com/…"
        );
        assert_eq!(
            redact_url("https://user:pass@sub.example.com:8443/api/sub?token=abc#frag"),
            "https://sub.example.com:8443/…"
        );
        assert_eq!(redact_url("http://host/"), "http://host");
        assert_eq!(redact_url("https://host/path"), "https://host/…");
        // 解析失败不泄露原始串
        assert_eq!(redact_url("not a url \n bad"), "***");
    }

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

    #[test]
    fn temp_paths_are_unique_and_suffixed() {
        let dir = TempDir::new("tmpname");
        let target = dir.path("sub.yaml");
        let a = temp_path_for(&target);
        let b = temp_path_for(&target);
        assert_ne!(a, b);
        let name = a.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with("sub.yaml.tmp."));
        assert!(a.parent() == target.parent(), "temp 必须与目标同目录");
    }

    #[test]
    fn commit_replaces_file_and_keeps_backup() {
        let dir = TempDir::new("commit-ok");
        let target = dir.path("sub.yaml");
        std::fs::write(&target, b"old content").unwrap();
        let temp = temp_path_for(&target);
        std::fs::write(&temp, b"new content").unwrap();

        commit_profile_file(&temp, &target).unwrap();

        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new content");
        // 旧内容保留在 .bak，临时文件已被消费（rename 走）不再存在
        assert_eq!(
            std::fs::read_to_string(backup_path_for(&target)).unwrap(),
            "old content"
        );
        assert!(!temp.exists());
    }

    #[test]
    fn commit_failure_restores_backup_and_cleans_temp() {
        let dir = TempDir::new("rollback");
        let target = dir.path("sub.yaml");
        std::fs::write(&target, b"original").unwrap();
        // 临时文件不存在 → 第二步 rename 必然失败，触发回滚
        let temp = temp_path_for(&target);

        assert!(commit_profile_file(&temp, &target).is_err());

        // .bak 已恢复原位：原订阅内容完好，无残留备份/临时文件
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "original");
        assert!(!backup_path_for(&target).exists());
        assert!(!temp.exists());
    }

    // ---- D6：启动时一次性订阅静默刷新 ----

    /// 阈值逻辑：仅「含订阅头 + mtime 超过 24h」入选；本地配置与 mtime 未知跳过
    #[test]
    fn select_stale_filters_by_url_and_age() {
        let now = std::time::SystemTime::now();
        let fresh = now - std::time::Duration::from_secs(3600); // 1h 前：新鲜
        let stale = now - std::time::Duration::from_secs(25 * 3600); // 25h 前：过期
        let candidates = vec![
            ("fresh-sub".to_string(), true, Some(fresh)),
            ("stale-sub".to_string(), true, Some(stale)),
            ("stale-local".to_string(), false, Some(stale)), // 无订阅头
            ("no-mtime".to_string(), true, None),            // mtime 读取失败
        ];
        let out = select_stale_subscriptions(candidates, now);
        assert_eq!(out, vec!["stale-sub".to_string()]);
    }

    /// 边界：恰好 24h（等于阈值）不刷新，超过 1 秒才刷新；未来 mtime 不刷新
    #[test]
    fn select_stale_boundary_at_24h() {
        let now = std::time::SystemTime::now();
        let exact = now - SUBSCRIPTION_REFRESH_AGE;
        let over = now - SUBSCRIPTION_REFRESH_AGE - std::time::Duration::from_secs(1);
        let future = now + std::time::Duration::from_secs(600);
        let candidates = vec![
            ("exact".to_string(), true, Some(exact)),
            ("over".to_string(), true, Some(over)),
            ("future".to_string(), true, Some(future)),
        ];
        let out = select_stale_subscriptions(candidates, now);
        assert_eq!(out, vec!["over".to_string()]);
    }
}
