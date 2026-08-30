// src-tauri/src/commands/config.rs
//! 配置命令：获取/更新/重置/导入/导出配置
//!
//! 事务化设计：
//! update / reset / import 的变更统一经 `core::app_controller::AppController`
//! 提交（事务串行锁在控制器内部获取，调用方无法绕过），事务链为：
//!
//! ```text
//! 快照旧配置 → 快照 Windows 代理状态 → 校验新配置 → 持久化(disk-first)
//!   → 重写 runtime-config → 热重载/重启运行中的核心（含健康检查）
//!   → 同步 Windows 系统代理副作用 → commit
//! ```
//!
//! Windows 系统代理纳入同一事务——
//! 任何一步失败都会把磁盘、内存、Mihomo 运行时、Windows 注册表恢复到
//! 操作前状态；成功返回等价于「UI = Config = runtime-config =
//! Mihomo 实际监听 = Windows 系统代理」五态一致。
//!
//! 更新/重置/导入都会改变托盘菜单展示的配置项（模式/系统代理/TUN/混合/
//! 激活 Profile/语言），因此命令完成后再刷新托盘菜单勾选态与文案。

use crate::config::model::Config;
use crate::util::error::{Error, Result};
use tauri::{command, AppHandle, State};
use tracing::info;

#[command]
pub async fn get_config(state: State<'_, crate::AppState>) -> Result<serde_json::Value> {
    let config_guard = state.config_manager.lock().unwrap();
    let mut value = serde_json::to_value(config_guard.get_config())?;
    // 控制器密钥不得返回 WebView。
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

/// 降级模式用户可见提示：config.yaml 损坏且迁移失败时，应用以默认配置
/// 降级运行，UI 必须明确告知（原文件已备份到何处、保存将覆盖原文件）。
/// 返回 { degraded, backup_file, message }；正常时 degraded=false。
#[command]
pub async fn get_config_degraded(state: State<'_, crate::AppState>) -> Result<serde_json::Value> {
    let (degraded, backup) = {
        let config_guard = state.config_manager.lock().unwrap();
        let degraded = config_guard.is_degraded();
        let backup = if degraded {
            crate::config::persistence::find_corrupt_backup(&config_guard.config_path())
        } else {
            None
        };
        (degraded, backup)
    };
    Ok(serde_json::json!({
        "degraded": degraded,
        "backup_file": backup,
        "message": if degraded {
            "检测到 config.yaml 损坏且自动迁移失败，应用正在使用默认配置运行（降级模式）。\n原文件未被覆盖，已备份为 config.yaml.corrupt-*.bak；只有在你确认备份位置并勾选「我确认覆盖损坏的配置文件」后，保存才会写入新配置。"
                .to_string()
        } else {
            String::new()
        }
    }))
}

/// 降级模式下普通保存的守卫：未显式确认（acknowledge_corrupt_config=true）
/// 时拒绝保存，防止静默覆盖损坏的原配置。整包替换语义的命令
/// （reset_config / import_config）是用户的显式破坏性操作，不在此守卫范围。
fn require_save_allowed_when_degraded(
    state: &State<'_, crate::AppState>,
    acknowledge_corrupt_config: Option<bool>,
) -> Result<()> {
    let degraded = state.config_manager.lock().unwrap().is_degraded();
    if degraded && !acknowledge_corrupt_config.unwrap_or(false) {
        return Err(Error::Other(
            "检测到 config.yaml 损坏，应用正以默认配置降级运行。为保护你的数据，\
             原文件（已备份为 config.yaml.corrupt-*.bak）不会被静默覆盖。\
             请确认备份位置后在设置页勾选「我确认覆盖损坏的配置文件」再保存。"
                .to_string(),
        ));
    }
    Ok(())
}

#[command]
pub async fn update_config(
    app: AppHandle,
    state: State<'_, crate::AppState>,
    config: serde_json::Value,
    acknowledge_corrupt_config: Option<bool>,
) -> Result<()> {
    // 降级守卫：损坏配置未经确认不得被普通保存静默覆盖。
    require_save_allowed_when_degraded(&state, acknowledge_corrupt_config)?;
    let new_config = {
        let config_guard = state.config_manager.lock().unwrap();
        config_guard.prepare_update(config)?
    };
    state.controller.update_config(&app, new_config).await?;
    crate::core::runtime::refresh_tray(&app).await
}

/// 字段级更新：前端只提交发生变化的顶层键（kebab-case），
/// 后端浅合并到当前配置后再走与 update_config 完全相同的校验 + 事务。
/// 消除整包回传的读-改-写竞态——用户停留在设置页期间托盘/其他入口
/// 改过的字段不会再被旧快照覆盖。
///
/// 并发正确性：读取+合并必须发生在持锁之后——若在加锁前读取
/// 并合并当前配置，另一个事务可能在"读取后、加锁前"完成提交，随后被本事务
/// 的旧快照整包覆盖。现在读取+合并移入 `AppController::update_config_fields`，
/// 在持有事务锁之后重新读取最新配置再合并，保证不同字段的并发更新
/// 互不覆盖。
#[command]
pub async fn update_config_fields(
    app: AppHandle,
    state: State<'_, crate::AppState>,
    patch: serde_json::Value,
    acknowledge_corrupt_config: Option<bool>,
) -> Result<()> {
    // 降级守卫：损坏配置未经确认不得被普通保存静默覆盖。
    require_save_allowed_when_degraded(&state, acknowledge_corrupt_config)?;
    let obj = patch
        .as_object()
        .ok_or_else(|| Error::InvalidArgument("patch 必须是顶层键值对象".to_string()))?;
    if obj.is_empty() {
        return Ok(());
    }
    state.controller.update_config_fields(&app, obj).await?;
    crate::core::runtime::refresh_tray(&app).await
}

#[command]
pub async fn reset_config(app: AppHandle, state: State<'_, crate::AppState>) -> Result<()> {
    // set_config 内部会把默认占位密钥轮换为随机值
    state
        .controller
        .update_config(&app, Config::default())
        .await?;
    crate::core::runtime::refresh_tray(&app).await
}

/// 设置页「从文件导入 YAML」的文件选择与读取全部收口到 Rust 侧。
/// 前端不再传任意绝对路径（WebView 被攻破时可借此遍历读磁盘 YAML），
/// 改为由 Rust 侧弹出系统文件对话框，用户选定后立即校验扩展名与大小上限
/// 并读取内容返回；取消选择返回 None。
#[command]
pub async fn pick_import_file(app: AppHandle) -> Result<Option<String>> {
    // blocking_pick_file 会阻塞当前线程，不能在 async 上下文/主线程调用，
    // 放到独立的阻塞线程池线程执行。
    let picked = tokio::task::spawn_blocking(move || {
        use tauri_plugin_dialog::DialogExt;
        app.dialog()
            .file()
            .add_filter("YAML", &["yaml", "yml"])
            .blocking_pick_file()
    })
    .await
    .map_err(|e| Error::Other(format!("文件对话框任务失败：{}", e)))?;

    let Some(path) = picked else {
        return Ok(None); // 用户取消
    };
    let path = path
        .into_path()
        .map_err(|e| Error::Other(format!("无效的文件路径：{}", e)))?;

    // 对话框已按扩展名过滤，仍需防御性校验（对话框可被绕过输入任意路径）
    let ext_ok = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| matches!(e.to_ascii_lowercase().as_str(), "yaml" | "yml"))
        .unwrap_or(false);
    if !ext_ok {
        return Err(Error::InvalidArgument(
            "仅支持导入 .yaml / .yml 配置文件".to_string(),
        ));
    }
    if !path.is_file() {
        return Err(Error::NotFound(format!("文件不存在：{}", path.display())));
    }
    const MAX_IMPORT_BYTES: u64 = 10 * 1024 * 1024;
    let meta = std::fs::metadata(&path)?;
    if meta.len() > MAX_IMPORT_BYTES {
        return Err(Error::InvalidArgument(
            "配置文件超过 10 MB 大小限制".to_string(),
        ));
    }
    std::fs::read_to_string(&path)
        .map(Some)
        .map_err(|e| Error::InvalidArgument(format!("读取文件失败：{}", e)))
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
/// （全程事务，任一步失败回滚到操作前状态并返回 Err）。
///
/// 导入语义修正（"导入配置不起效"）：build_runtime_config 中激活 Profile 的
/// `proxies` 优先于 AppConfig.extra 的 `proxies`——若用户此前激活过任何
/// 订阅 Profile，导入的完整 mihomo 配置节点会被旧订阅静默遮蔽，界面无任何
/// 变化。因此当导入 YAML 自带非空顶层 `proxies` 且未显式携带 `profile:` 键时，
/// 视为用户显式更换节点来源，重置激活 Profile 为内置 DIRECT，让导入节点生效。
#[command]
pub async fn import_config(
    app: AppHandle,
    state: State<'_, crate::AppState>,
    yaml: String,
) -> Result<()> {
    let mut new_config = {
        let config_guard = state.config_manager.lock().unwrap();
        config_guard.prepare_import(yaml)?
    };
    if import_supplies_nodes(&new_config) {
        info!(
            "Import carries its own proxies; resetting active profile '{}' to builtin DIRECT \
             so imported nodes take effect",
            new_config.general.profile
        );
        // 空 profile → read_active_profile 返回 None → build_runtime_config
        // 走 extra.proxies 分支；下次启动 init/merge_rules 会归一为 "DIRECT"。
        new_config.general.profile = String::new();
    }
    state.controller.update_config(&app, new_config).await?;
    crate::core::runtime::refresh_tray(&app).await
}

/// 判断导入配置是否自带生效节点来源：
/// - 顶层 `proxies` 非空列表 → true；
/// - 显式携带 `profile:` 键 → false（尊重导入配置自身的 Profile 指向，
///   例如 ClashEdge 导出的完整状态），由该 Profile 提供节点。
fn import_supplies_nodes(config: &Config) -> bool {
    if !config.general.profile.is_empty() {
        return false;
    }
    config
        .extra
        .get("proxies")
        .and_then(|v| v.as_sequence())
        .is_some_and(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::persistence::ConfigManager;

    /// 字段级更新必须在持有配置事务锁后重读最新配置再合并。
    /// 这里用 tokio Mutex 模拟 AppController 内部事务锁的串行语义，两个并发字段
    /// 更新（分别改顶层 `mixed-port` 与 `locale`）在串行事务下必须互不覆盖。
    /// 若"锁外读快照、锁内整包提交"，后完成的事务会用旧快照覆盖先完成的字段。
    #[tokio::test]
    async fn concurrent_field_updates_preserve_each_other() {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;

        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "clashedge-cfg-fields-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let mgr = Arc::new(std::sync::Mutex::new({
            let mut m = ConfigManager::new();
            m.init(&dir).unwrap();
            m
        }));
        // 模拟 AppController 内部的事务串行锁：跨 await 持有、串行所有配置事务。
        let tx = Arc::new(tokio::sync::Mutex::new(()));

        // 事务内"重读最新配置 + 合并 patch + 校验"——与修复后的
        // commit_config_fields_transaction 逻辑一致。
        async fn apply_field(
            mgr: &Arc<std::sync::Mutex<ConfigManager>>,
            tx: &Arc<tokio::sync::Mutex<()>>,
            patch: serde_json::Value,
        ) {
            let _g = tx.lock().await;
            let merged = {
                let guard = mgr.lock().unwrap();
                let mut current = serde_json::to_value(guard.get_config()).unwrap();
                let cur_obj = current.as_object_mut().unwrap();
                for (k, v) in patch.as_object().unwrap() {
                    cur_obj.insert(k.clone(), v.clone());
                }
                guard.prepare_update(current).unwrap()
            };
            mgr.lock().unwrap().set_config(merged).unwrap();
        }

        // 两个不同字段的并发更新（乱序完成也互不覆盖）
        let mgr_a = Arc::clone(&mgr);
        let tx_a = Arc::clone(&tx);
        let patch_a = serde_json::json!({ "mixed-port": 7899 });
        let h_a = tokio::spawn(async move { apply_field(&mgr_a, &tx_a, patch_a).await });
        let mgr_b = Arc::clone(&mgr);
        let tx_b = Arc::clone(&tx);
        let patch_b = serde_json::json!({ "locale": "en-US" });
        let h_b = tokio::spawn(async move { apply_field(&mgr_b, &tx_b, patch_b).await });
        h_a.await.unwrap();
        h_b.await.unwrap();

        let final_cfg = mgr.lock().unwrap().get_config();
        assert_eq!(
            final_cfg.general.mixed_port, 7899,
            "first field update (mixed-port) must survive the second transaction"
        );
        assert_eq!(
            final_cfg.locale, "en-US",
            "second field update (locale) must not be clobbered"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn config_with_extra(extra_yaml: &str) -> Config {
        let mut config = Config::default();
        let value: serde_yaml::Value = serde_yaml::from_str(extra_yaml).unwrap();
        if let serde_yaml::Value::Mapping(map) = value {
            for (k, v) in map {
                config.extra.insert(k, v);
            }
        }
        config
    }

    /// 导入配置自带非空顶层 proxies 且无 profile 键 → 视为节点来源，
    /// 应重置激活 Profile 让导入节点生效（"导入配置不起效"修复）。
    #[test]
    fn import_supplies_nodes_true_for_bare_proxies() {
        let config = config_with_extra(
            "proxies:\n  - name: N1\n    type: ss\n    server: 1.1.1.1\n    port: 8388\n",
        );
        assert!(import_supplies_nodes(&config));
    }

    #[test]
    fn import_supplies_nodes_respects_explicit_profile_key() {
        let mut config = config_with_extra(
            "proxies:\n  - name: N1\n    type: ss\n    server: 1.1.1.1\n    port: 8388\n",
        );
        config.general.profile = "MySub".to_string();
        assert!(!import_supplies_nodes(&config));
    }

    #[test]
    fn import_supplies_nodes_false_for_empty_or_missing_proxies() {
        // 空 proxies 列表：无生效节点，不触发 Profile 重置
        let config = config_with_extra("proxies: []\n");
        assert!(!import_supplies_nodes(&config));
        // 无 proxies 键
        assert!(!import_supplies_nodes(&Config::default()));
    }
}
