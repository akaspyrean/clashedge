// src-tauri/src/commands/config.rs
//! 配置命令：获取/更新/重置/导入/导出配置
//!
//! P0-3/P0-4 事务化（AUDIT-0.8.7）：
//! update / reset / import 统一走 `commit_config_transaction`：
//!
//! ```text
//! 快照旧配置 → 校验新配置 → 持久化(disk-first) → 重写 runtime-config
//!   → 热重载/重启运行中的核心 → 失败则回滚持久化并恢复运行时 → 返回 Err
//! ```
//!
//! 任何一步失败都会把磁盘、内存、Mihomo 运行时恢复到操作前状态；
//! UI 的"保存成功"因此等价于「磁盘 + Mihomo 实际状态」一致。
//!
//! 更新/重置/导入都会改变托盘菜单展示的配置项（模式/系统代理/TUN/混合/
//! 激活 Profile/语言），因此命令完成后再刷新托盘菜单勾选态与文案。

use crate::config::model::Config;
use crate::util::error::{Error, Result};
use tauri::{command, AppHandle, State};
use tracing::{error, warn};

#[command]
pub async fn get_config(state: State<'_, crate::AppState>) -> Result<serde_json::Value> {
    let config_guard = state.config_manager.lock().unwrap();
    let mut value = serde_json::to_value(config_guard.get_config())?;
    // P0-3：控制器密钥不得返回 WebView。
    // 内部配置继续保存真实 secret，Rust 调用 Mihomo API 的 Bearer 鉴权
    // 直接读共享配置 Arc（api_headers），不受此处脱敏影响。
    // 前端拿到的 secret 替换为脱敏占位符；update_config 见到脱敏值时保留
    // 现有真实密钥，不轮换（避免每次保存都换密钥导致运行中 mihomo 鉴权失效）。
    if let Some(obj) = value.as_object_mut() {
        if obj.contains_key("secret") {
            obj.insert(
                "secret".to_string(),
                serde_json::Value::String(crate::config::model::SECRET_REDACTED.to_string()),
            );
        }
    }
    Ok(value)
}

#[command]
pub async fn update_config(
    app: AppHandle,
    state: State<'_, crate::AppState>,
    config: serde_json::Value,
) -> Result<()> {
    let new_config = {
        let config_guard = state.config_manager.lock().unwrap();
        config_guard.prepare_update(config)?
    };
    commit_config_transaction(&state, new_config).await?;
    crate::core::runtime::refresh_tray(&app).await
}

#[command]
pub async fn reset_config(app: AppHandle, state: State<'_, crate::AppState>) -> Result<()> {
    // set_config 内部会把默认占位密钥轮换为随机值（H1）
    commit_config_transaction(&state, Config::default()).await?;
    crate::core::runtime::refresh_tray(&app).await
}

/// P1-13：设置页「从文件导入 YAML」的文件读取收口到 Rust 侧。
/// 前端只传用户在系统对话框中选择的路径，这里校验扩展名（.yaml/.yml）
/// 与大小上限（10 MB）后读取内容返回——不给 WebView 开放通用 fs 读权限，
/// capability 保持只有 dialog 权限。
#[command]
pub async fn read_import_file(path: String) -> Result<String> {
    let p = std::path::Path::new(&path);
    let ext_ok = p
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| matches!(e.to_ascii_lowercase().as_str(), "yaml" | "yml"))
        .unwrap_or(false);
    if !ext_ok {
        return Err(Error::InvalidArgument(
            "仅支持导入 .yaml / .yml 配置文件".to_string(),
        ));
    }
    if !p.is_file() {
        return Err(Error::NotFound(format!("文件不存在：{}", path)));
    }
    const MAX_IMPORT_BYTES: u64 = 10 * 1024 * 1024;
    let meta = std::fs::metadata(p)?;
    if meta.len() > MAX_IMPORT_BYTES {
        return Err(Error::InvalidArgument(
            "配置文件超过 10 MB 大小限制".to_string(),
        ));
    }
    std::fs::read_to_string(p).map_err(|e| Error::InvalidArgument(format!("读取文件失败：{}", e)))
}

/// 事务式提交配置变更：校验已完成，这里执行「持久化 → 应用运行时 → 失败回滚」。
async fn commit_config_transaction(
    state: &State<'_, crate::AppState>,
    new_config: Config,
) -> Result<()> {
    // 1. 快照旧配置（回滚基准）
    let old = { state.config_manager.lock().unwrap().get_config() };

    // 2. 持久化新配置（disk-first：落盘成功才提交内存）
    {
        let mut guard = state.config_manager.lock().unwrap();
        guard.set_config(new_config)?;
    }

    // 3. 应用到运行时：重写 runtime-config.yaml + 热重载/重启运行中的核心。
    //    核心未运行时 reload_running_core 只重写文件，不会失败于此路径之外。
    if let Err(e) = reload_running_core(state).await {
        error!("Config change failed to apply ({}); rolling back", e);

        // 4a. 回滚持久化（内存 + 磁盘恢复旧值）
        {
            let mut guard = state.config_manager.lock().unwrap();
            guard.set_config(old).map_err(|rb| {
                error!("Rollback persist failed: {}", rb);
                rb
            })?;
        }

        // 4b. 尽力把运行时也拉回旧配置（失败不掩盖原始错误）
        if let Err(rb) = reload_running_core(state).await {
            warn!("Rollback runtime restore failed: {}", rb);
        }

        return Err(Error::Other(format!("配置已保存但应用失败，已回滚：{}", e)));
    }

    Ok(())
}

/// 导出当前完整配置为一份可直接使用的 mihomo 配置文件。
///
/// 产物 = `build_runtime_config`（应用设置 + 激活 Profile 合并），而非应用内部
/// AppConfig——用户期望「导出配置」得到的是能拿去给其他 Clash 客户端加载的
/// 配置文件（含节点 / 分组 / 规则）。文件自动生成到数据目录 config-export.yaml，
/// 返回文件路径（前端提示即可，不再弹保存对话框）。
#[command]
pub async fn export_config(app: AppHandle, state: State<'_, crate::AppState>) -> Result<String> {
    // 读共享配置与激活 Profile 内容
    let (config, profile_content) = {
        let config_guard = state.config_manager.lock().unwrap();
        let config = config_guard.get_config();
        let profile = config.general.profile.clone();
        let content = if profile.is_empty() {
            None
        } else {
            let dir = crate::util::paths::get_app_data_dir(&app)?.join("profiles");
            let path = crate::util::paths::sanitize_profile_name(&profile)
                .map(|safe| dir.join(format!("{}.yaml", safe)))
                .ok()
                .filter(|p| p.exists());
            path.and_then(|p| std::fs::read_to_string(p).ok())
        };
        (config, content)
    };

    // 生成运行时配置并落盘
    let mut runtime =
        crate::core::config::build_runtime_config(&config, profile_content.as_deref())?;
    // 导出脱敏：控制器密钥替换为占位符，避免导出文件泄露真实 secret。
    // 注意：节点密码（proxies 内 password/uuid 等）仍是敏感信息，导出后需妥善保管。
    if let Some(map) = runtime.as_mapping_mut() {
        map.insert(
            serde_yaml::Value::String("secret".to_string()),
            serde_yaml::Value::String("********".to_string()),
        );
    }
    let yaml = serde_yaml::to_string(&runtime)?;
    let data_dir = crate::util::paths::get_app_data_dir(&app)?;
    let export_path = data_dir.join("config-export.yaml");
    crate::util::atomic::atomic_write(&export_path, yaml.as_bytes())?;
    Ok(export_path.to_string_lossy().to_string())
}

/// 从 YAML 导入配置并使其生效：解析 → 校验 → 落盘 → 重建运行时 → 重载核心
/// （P0-4：全程事务，任一步失败回滚到操作前状态并返回 Err）。
#[command]
pub async fn import_config(
    app: AppHandle,
    state: State<'_, crate::AppState>,
    yaml: String,
) -> Result<()> {
    let new_config = {
        let config_guard = state.config_manager.lock().unwrap();
        config_guard.prepare_import(yaml)?
    };
    commit_config_transaction(&state, new_config).await?;
    crate::core::runtime::refresh_tray(&app).await
}

/// 重建运行时配置并对运行中的核心生效（热重载，失败回退整进程重启）。
///
/// P0-4：错误必须向上传播——旧实现把 reload 失败吞成 warn 日志后返回成功，
/// 导致「新配置已写盘但 Mihomo 仍用旧值」的假成功。核心未运行时不报错：
/// 文件已重写，下次启动自然加载新配置。
async fn reload_running_core(state: &State<'_, crate::AppState>) -> Result<()> {
    let core_guard = state.core_manager.lock().await;
    if let Some(core) = core_guard.as_ref() {
        core.reload_config().await?;
    }
    Ok(())
}
