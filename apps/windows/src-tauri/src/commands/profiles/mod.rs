// src-tauri/src/commands/profiles/mod.rs
//! 配置文件命令：列表、新建、删除、重命名、激活、编辑、导入、导出
//!
//! 安全：所有 `profiles/<name>.yaml` 路径构造必须先过
//! `util::paths::sanitize_profile_name`（防路径穿越），
//! 否则 `name = "..\\..\\config.yaml"` 会越权读写任意文件。
//! 一致性：激活走统一编排层 `core::app_controller::AppController::activate_profile`
//! （校验 → 持久化 → 重生成运行时配置 → 热重载核心 → 失败回滚），
//! 删除/重命名激活中的 Profile 时同步修正激活标记。
//! rename / update_profile_content / 订阅提交的「文件变更 + 激活」复合事务
//! 主体在 `*_locked` 函数中，由 AppController 持有事务锁后调用。
//!
//! 模块拆分（按单一职责）：
//! - `validate`     —— 订阅内容资源限制与协议字段校验；
//! - `files`        —— profile 文件路径/临时文件/事务式替换/激活回滚；
//! - `subscription` —— 订阅 URL 提取/脱敏、下载、归一化、静默刷新。
//!
//! 公开路径（`crate::commands::profiles::<command>` 与
//! `auto_refresh_stale_subscriptions` / `refresh_subscription`）保持不变。

use std::path::Path;

use tauri::{command, AppHandle, Manager};
use tracing::{info, warn};

use crate::util::atomic::atomic_write;
use crate::util::error::{Error, Result};
use crate::util::paths::get_profiles_dir;

mod files;
mod subscription;
mod validate;

pub use subscription::{auto_refresh_stale_subscriptions, refresh_subscription};

use files::{
    activate_with_rollback, active_profile, commit_profile_file, pending_delete_path_for,
    profile_path, reserve_new, temp_path_for,
};
use subscription::{
    apply_header_user_agent, build_header, download_subscription_streaming, extract_subscribe_url,
    extract_subscribe_user_agent, extract_subscription_userinfo, normalize_subscription_body,
    read_profile_head, redact_subscribe_url, redact_url, sanitize_user_agent,
    strip_subscribe_header,
};
use validate::{validate_profile_strict, validate_subscription_content};

/// 重命名 Profile 复合事务主体（调用方 AppController 已持有事务锁）。
///
/// rename / activate / rollback 都走 *_locked 变体，避免嵌套取锁死锁。
pub(crate) async fn rename_profile_locked(
    app: &AppHandle,
    old_name: &str,
    new_name: &str,
    old_path: &Path,
    new_path: &Path,
) -> Result<()> {
    let was_active = active_profile(app) == old_name;
    std::fs::rename(old_path, new_path)?;

    // 非激活 profile：仅重命名文件即可。
    if !was_active {
        return Ok(());
    }

    // 激活中的 profile：事务化重命名。
    //  1) rename 文件 old → new（已在上方完成）
    //  2) activate_profile_locked(new)：持久化 config.profile=new + 重生成运行时 + 重启核心
    //     （成功：config=file=core 三者一致，并已刷新托盘/推送事件）
    //  3) 失败：activate_profile_locked 已把 config.profile 回滚到旧名，这里再把
    //     文件 new → old 恢复，并重新激活旧名让核心加载回旧文件。
    if let Err(e) = crate::core::runtime::activate_profile_locked(app, new_name).await {
        // 回滚：config.profile 已被 activate_profile_locked 回滚到旧名；这里恢复文件。
        let restored = std::fs::rename(new_path, old_path).is_ok();
        if let Err(e2) = crate::core::runtime::activate_profile_locked(app, old_name).await {
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

/// 保存 Profile 内容的复合事务主体（调用方 AppController 已持有事务锁，
/// 文件变化不在事务锁外）。
///
/// 事务式写：旧文件 → .bak，新内容 → target；激活中的 Profile 保存后立即生效，
/// 激活失败回滚 .bak 旧内容（避免"保存成功但运行还是旧节点"的界面≠运行不一致）。
pub(crate) async fn update_profile_content_locked(
    app: &AppHandle,
    name: &str,
    file_path: &Path,
    content: &str,
) -> Result<()> {
    let temp_path = temp_path_for(file_path);
    std::fs::write(&temp_path, content.as_bytes())?;
    commit_profile_file(&temp_path, file_path)?;

    if active_profile(app) == name {
        activate_with_rollback(app, name, file_path).await?;
    }

    Ok(())
}

/// 订阅更新提交的复合事务主体（调用方 AppController 已持有事务锁）：
/// 覆写临时文件 → 备份旧文件为 .bak，再把临时文件 rename 为正式文件
/// （任一步失败自动恢复 .bak 并清理临时文件）→ 激活中的 Profile 热重载生效。
pub(crate) async fn commit_subscription_update_locked(
    app: &AppHandle,
    name: &str,
    file_path: &Path,
    temp_path: &Path,
    final_text: &str,
) -> Result<()> {
    std::fs::write(temp_path, final_text.as_bytes())?;
    commit_profile_file(temp_path, file_path)?;

    // 更新激活中的 Profile：热重载使新节点/规则生效；激活失败回滚 .bak 旧内容。
    if active_profile(app) == name {
        activate_with_rollback(app, name, file_path).await?;
    }

    Ok(())
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
                // 只读文件开头（头注释所在），不整文件读入
                let head = read_profile_head(&path);
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
                    // 订阅地址脱敏——前端只获得 host 或脱敏 URL，
                    // token/key 不返回前端。后端更新功能读回文件内完整 URL 重新拉取。
                    "url": extract_subscribe_url(&head)
                        .and_then(|u| redact_subscribe_url(&u)),
                    // 订阅流量 / 到期信息（来自 Subscription-Userinfo 响应头）
                    "userinfo": extract_subscription_userinfo(&head),
                    // 单个订阅的自定义 User-Agent（# subscribe-user-agent: 注释头，
                    // 审计 B7）；无则为 null（用内置 mihomo 兼容 UA）
                    "user_agent": extract_subscribe_user_agent(&head),
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
        validate_profile_strict(c)?;
    }

    // 空内容走内置模板。产品架构里 Profile 只提供节点（runtime 仅透传 proxies），
    // 因此空模板就是空节点集，而非一份"看起来像完整 Mihomo 配置、实际大部分字段
    // 不生效"的误导性模板。
    let yaml = content.unwrap_or_else(|| "proxies: []\n".to_string());

    reserve_new(&file_path)?;
    if let Err(e) = atomic_write(&file_path, yaml.as_bytes()) {
        let _ = std::fs::remove_file(&file_path);
        return Err(e);
    }
    Ok(())
}

#[command]
pub async fn delete_profile(app: AppHandle, name: String) -> Result<()> {
    let profiles_dir = get_profiles_dir(&app)?;
    let file_path = profile_path(&profiles_dir, &name)?;

    if !file_path.exists() {
        return Err(Error::NotFound("Profile not found".to_string()));
    }
    // 整个删除事务（含"是否激活中"的判断）在 AppController 事务锁内完成，
    // 避免判断后、操作前被别的事务切换激活态。
    let state = app.state::<crate::AppState>();
    state
        .controller
        .delete_profile(&app, &name, &file_path)
        .await
}

/// 删除 Profile 复合事务主体（调用方 AppController 已持有事务锁）。
///
/// 事务化删除：先把正式文件暂存为 `.pending-delete`，切换激活成功后才真正删除。
/// 若切换激活失败，恢复暂存文件，保证原 profile 不丢失（避免"文件已删但激活态
/// 仍指向它"的不一致）。
pub(crate) async fn delete_profile_locked(
    app: &AppHandle,
    name: &str,
    file_path: &Path,
) -> Result<()> {
    let was_active = active_profile(app) == name;
    let pending = pending_delete_path_for(file_path);
    std::fs::rename(file_path, &pending)?;

    // 删除的是激活中的 Profile：先重置回内置预设 DIRECT 并重载核心；失败恢复文件。
    if was_active {
        if let Err(e) = crate::core::runtime::activate_profile_locked(app, "DIRECT").await {
            let _ = std::fs::rename(&pending, file_path);
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

    let state = app.state::<crate::AppState>();
    state
        .controller
        .rename_profile(&app, &old_name, &new_name, &old_path, &new_path)
        .await
}

#[command]
pub async fn activate_profile(app: AppHandle, name: String) -> Result<()> {
    let state = app.state::<crate::AppState>();
    state.controller.activate_profile(&app, &name).await
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

    // 手动编辑：严格校验（节点名唯一 / 非保留名）
    validate_profile_strict(&content)?;

    let state = app.state::<crate::AppState>();
    state
        .controller
        .update_profile_content(&app, &name, &file_path, &content)
        .await
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
    let (normalized, _warnings) = normalize_subscription_body(&app, &name, &content).await?;
    let normalized = format!("# profile: {}\n{}", name, normalized);
    validate_profile_strict(&normalized)?;

    reserve_new(&file_path)?;
    if let Err(e) = atomic_write(&file_path, normalized.as_bytes()) {
        let _ = std::fs::remove_file(&file_path);
        return Err(e);
    }
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
pub async fn import_profile_from_url(
    app: AppHandle,
    name: String,
    url: String,
    user_agent: Option<String>,
) -> Result<()> {
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
    // 单个订阅可覆盖 User-Agent（审计 B7）：净化后随头注释持久保留，
    // 空/全不可用回落到内置 mihomo 兼容 UA
    let custom_ua = user_agent.as_deref().and_then(sanitize_user_agent);

    // C2 SSRF 防护：parse+scheme 校验后再做禁段校验（localhost/.local/回环/私网等）
    crate::util::fetch::validate_url_app(&app, &url).await?;

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
    // RAII：任何提前返回（含 `?`）都会清理临时文件
    let _temp_guard = crate::util::temp::TempFile::new(temp_path.clone());
    let header = build_header(parsed.as_str(), None, custom_ua.as_deref());
    let userinfo =
        download_subscription_streaming(&app, &url, &header, &temp_path, custom_ua.as_deref())
            .await?;

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
    let (normalized, warnings) = normalize_subscription_body(&app, &name, &body).await?;
    if !warnings.is_empty() {
        warn!("Import '{}': {}", redact_url(&url), warnings.join("；"));
    }
    let final_text = format!(
        "{}{}",
        build_header(parsed.as_str(), userinfo.as_deref(), custom_ua.as_deref()),
        normalized
    );
    if let Err(e) = validate_profile_strict(&final_text) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(e);
    }
    // 归一化结果覆写临时文件后，再原子提交为正式文件
    std::fs::write(&temp_path, final_text.as_bytes())?;

    // 新文件：先原子占位文件名（并发创建同名 profile 时后到者报"已存在"），
    // 再把临时文件原子 rename 覆盖到正式位置。
    reserve_new(&file_path)?;
    if let Err(e) = commit_profile_file(&temp_path, &file_path) {
        let _ = std::fs::remove_file(&file_path);
        return Err(e);
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

/// 设置订阅自定义 User-Agent（审计 B7）。仅变更 profile 头注释元数据，
/// 不影响 mihomo 运行时配置（生成器只读 proxies），无需热重载。
/// `user_agent` 为空字符串或不可用字符 → 移除自定义行，回到内置默认 UA。
/// 复合事务在 AppController 持锁内执行，避免与并发的订阅刷新/重命名互相覆盖。
#[command]
pub async fn set_profile_user_agent(
    app: AppHandle,
    name: String,
    user_agent: Option<String>,
) -> Result<()> {
    let profiles_dir = get_profiles_dir(&app)?;
    let file_path = profile_path(&profiles_dir, &name)?;
    if !file_path.exists() {
        return Err(Error::NotFound("Profile not found".to_string()));
    }
    // 净化：空 / 全不可用 → None（移除自定义行）
    let custom_ua = user_agent.as_deref().and_then(sanitize_user_agent);
    let state = app.state::<crate::AppState>();
    state
        .controller
        .set_profile_user_agent(&app, &name, &file_path, custom_ua)
        .await
}

/// 设置自定义 User-Agent 的复合事务主体（调用方 AppController 已持事务锁）：
/// 读取内容 → 替换/插入/移除头注释行 → 临时文件事务式替换正式文件。
/// 不触发激活：头注释是 app 元数据，不属于 mihomo 运行时配置。
pub(crate) async fn set_profile_user_agent_locked(
    file_path: &Path,
    user_agent: Option<String>,
) -> Result<()> {
    let content = std::fs::read_to_string(file_path)?;
    let updated = apply_header_user_agent(&content, user_agent.as_deref());
    if updated == content {
        // 无变化：不触发文件写（保留 mtime，且不产生 .bak 残留）
        return Ok(());
    }
    let temp_path = temp_path_for(file_path);
    // RAII：任何提前返回都会清理临时文件
    let _guard = crate::util::temp::TempFile::new(temp_path.clone());
    std::fs::write(&temp_path, updated.as_bytes())?;
    commit_profile_file(&temp_path, file_path)?;
    Ok(())
}
