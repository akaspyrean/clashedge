// src-tauri/src/commands/config.rs
//! 配置命令：获取/更新/重置/导入/导出配置
//!
//! P0-3/P0-4 事务化（AUDIT-0.8.7）：
//! update / reset / import 统一走 `commit_config_transaction`：
//!
//! ```text
//! 快照旧配置 → 快照 Windows 代理状态 → 校验新配置 → 持久化(disk-first)
//!   → 重写 runtime-config → 热重载/重启运行中的核心（含健康检查）
//!   → 同步 Windows 系统代理副作用 → commit
//! ```
//!
//! P0-1（Release Gate）：Windows 系统代理纳入同一事务——
//! 任何一步失败都会把磁盘、内存、Mihomo 运行时、Windows 注册表恢复到
//! 操作前状态；成功返回等价于「UI = Config = runtime-config =
//! Mihomo 实际监听 = Windows 系统代理」五态一致。
//!
//! 更新/重置/导入都会改变托盘菜单展示的配置项（模式/系统代理/TUN/混合/
//! 激活 Profile/语言），因此命令完成后再刷新托盘菜单勾选态与文案。

use crate::config::model::Config;
use crate::proxy::journal::ProxyJournal;
use crate::proxy::system_proxy::SystemProxyConfig;
use crate::util::error::{Error, Result};
use tauri::{command, AppHandle, State};
use tracing::{error, info, warn};

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
    commit_config_transaction(&app, &state, new_config).await?;
    crate::core::runtime::refresh_tray(&app).await
}

/// 字段级更新：前端只提交发生变化的顶层键（kebab-case），
/// 后端浅合并到当前配置后再走与 update_config 完全相同的校验 + 事务。
/// 消除整包回传的读-改-写竞态——用户停留在设置页期间托盘/其他入口
/// 改过的字段不会再被旧快照覆盖。
#[command]
pub async fn update_config_fields(
    app: AppHandle,
    state: State<'_, crate::AppState>,
    patch: serde_json::Value,
) -> Result<()> {
    let obj = patch
        .as_object()
        .ok_or_else(|| Error::InvalidArgument("patch 必须是顶层键值对象".to_string()))?;
    if obj.is_empty() {
        return Ok(());
    }
    let new_config = {
        let config_guard = state.config_manager.lock().unwrap();
        let mut current = serde_json::to_value(config_guard.get_config())?;
        let cur_obj = current
            .as_object_mut()
            .ok_or_else(|| Error::Other("当前配置不是 JSON 对象".to_string()))?;
        for (k, v) in obj {
            cur_obj.insert(k.clone(), v.clone());
        }
        config_guard.prepare_update(current)?
    };
    commit_config_transaction(&app, &state, new_config).await?;
    crate::core::runtime::refresh_tray(&app).await
}

#[command]
pub async fn reset_config(app: AppHandle, state: State<'_, crate::AppState>) -> Result<()> {
    // set_config 内部会把默认占位密钥轮换为随机值（H1）
    commit_config_transaction(&app, &state, Config::default()).await?;
    crate::core::runtime::refresh_tray(&app).await
}

/// P1：设置页「从文件导入 YAML」的文件选择与读取全部收口到 Rust 侧。
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

/// 事务式提交配置变更：校验已完成，这里执行
/// 「持久化 → 应用运行时（含健康检查）→ Windows 副作用 → commit；
///   任一步失败 → Config / runtime-config / Mihomo / Windows 全部回滚」。
async fn commit_config_transaction(
    app: &AppHandle,
    state: &State<'_, crate::AppState>,
    new_config: Config,
) -> Result<()> {
    // 1. 快照旧配置（回滚基准；mixed_port 单独留存供 Windows 回滚使用）
    let old = { state.config_manager.lock().unwrap().get_config() };
    let old_port = old.general.mixed_port;

    // 2. P0-1：快照 Windows 系统代理状态（回滚基准；读取失败记为 None，
    //    回滚时退化为"仍指向本应用端口就关闭"的保守策略）
    let win_before = crate::proxy::system_proxy::get_system_proxy().ok();

    // 3. 持久化新配置（disk-first：落盘成功才提交内存）
    {
        let mut guard = state.config_manager.lock().unwrap();
        guard.set_config(new_config)?;
    }

    // 4. 应用到运行时：重写 runtime-config.yaml + 热重载/重启运行中的核心。
    //    P0-4：reload 成功与否由真实运行状态健康检查决定，不以 HTTP 200 为准。
    //    核心未运行时 reload_running_core 只重写文件，不会失败于此路径之外。
    if let Err(e) = reload_running_core(state).await {
        error!("Config change failed to apply ({}); rolling back", e);

        // 4a. 回滚持久化 + 运行时（内存 + 磁盘恢复旧值，再拉回旧运行态）
        if let Err(rb) = rollback_config_and_runtime(state, old).await {
            return Err(Error::Other(format!(
                "配置应用失败（{}），且配置回滚也失败：{}",
                e, rb
            )));
        }
        return Err(Error::Other(format!("配置已保存但应用失败，已回滚：{}", e)));
    }

    // 5. P0-1：Windows 副作用同步——注册表必须与新配置意图一致。
    //    失败则完整回滚四层状态，禁止出现「Config=new / runtime=new / Windows=old」。
    if let Err(e) = sync_windows_side_effects(app, state, win_before.as_ref()).await {
        error!(
            "Config change applied but Windows side-effect failed ({}); rolling back fully",
            e
        );
        let rb_err = match rollback_config_and_runtime(state, old).await {
            Ok(()) => None,
            Err(rb) => {
                error!(
                    "Config rollback during Windows side-effect failure also failed: {}",
                    rb
                );
                Some(rb)
            }
        };
        if let Err(we) = restore_windows_proxy(win_before.as_ref(), old_port) {
            error!("Windows proxy rollback failed: {}", we);
        }
        return Err(match rb_err {
            Some(rb) => Error::Other(format!(
                "系统代理同步失败（{}），已回滚；但配置回滚也失败：{}",
                e, rb
            )),
            None => Error::Other(format!("系统代理同步失败，已完整回滚：{}", e)),
        });
    }

    Ok(())
}

/// 回滚持久化（内存 + 磁盘恢复旧值）并尽力把 Mihomo 运行时拉回旧配置。
/// 持久化回滚失败 → Err；运行时恢复失败 → 记录后返回该错误（不掩盖原始错误）。
async fn rollback_config_and_runtime(
    state: &State<'_, crate::AppState>,
    old: Config,
) -> Result<()> {
    {
        let mut guard = state.config_manager.lock().unwrap();
        guard.set_config(old)?;
    }
    if let Err(rb) = reload_running_core(state).await {
        warn!("Rollback runtime restore failed: {}", rb);
        return Err(rb);
    }
    Ok(())
}

/// 尽力把 Windows 代理恢复到事务前快照。
/// 快照缺失（读取失败）时的保守策略：若注册表当前指向本应用旧端口则关闭，
/// 否则不动用户自己的代理。
fn restore_windows_proxy(snapshot: Option<&SystemProxyConfig>, old_port: u16) -> Result<()> {
    match snapshot {
        Some(s) => crate::proxy::system_proxy::set_system_proxy(
            s.enabled,
            &s.address,
            &s.bypass_list,
            s.auto_config_url.as_deref(),
        ),
        None => {
            let ours = format!("127.0.0.1:{}", old_port);
            let cur = crate::proxy::system_proxy::get_system_proxy().ok();
            if matches!(&cur, Some(c) if c.enabled && c.address == ours) {
                crate::proxy::system_proxy::set_system_proxy(false, "", &[], None)
            } else {
                Ok(())
            }
        }
    }
}

/// P0-1/P0-2：让 Windows 注册表与新配置的 system-proxy 意图一致。
///
/// - 新配置开启系统代理：先确保 Core Running 且 mixed-port 真实可连
///   （不开死代理），再把注册表指向 `127.0.0.1:<新 mixed-port>` 并维护 journal；
/// - 新配置关闭系统代理（import/reset 可能改变它）：把注册表还原为 journal /
///   快照记录的用户原始代理状态（无则关闭），并清除 journal。
async fn sync_windows_side_effects(
    app: &AppHandle,
    state: &State<'_, crate::AppState>,
    win_before: Option<&SystemProxyConfig>,
) -> Result<()> {
    let cfg = { state.config_manager.lock().unwrap().get_config() };
    let data_dir = crate::util::paths::get_app_data_dir(app)?;

    if cfg.general.system_proxy {
        // 开启（或 mixed-port 变更后重新指向）：先保证核心真实服务新端口
        crate::core::runtime::ensure_core_serving(app).await?;
        let address = format!("127.0.0.1:{}", cfg.general.mixed_port);
        crate::proxy::system_proxy::set_system_proxy(
            true,
            &address,
            &crate::core::runtime::default_bypass(),
            // 接管：删除用户原有 PAC（原值已随快照/journal 保留）
            None,
        )?;
        // journal：接管成功 → 记录"接管前"原始状态。已有 journal 时保留其
        // original（避免用我们自己写的代理覆盖用户真正的原始快照）。
        let existing_original = crate::proxy::journal::read_journal(&data_dir)
            .and_then(|j| j.original)
            .or_else(|| win_before.cloned());
        crate::proxy::journal::write_journal(
            &data_dir,
            &ProxyJournal {
                session_id: format!(
                    "{:016x}{:016x}",
                    rand::random::<u64>(),
                    rand::random::<u64>()
                ),
                pid: std::process::id(),
                mixed_port: cfg.general.mixed_port,
                original: existing_original,
                owned: true,
            },
        );
        info!("Windows system proxy synced to {}", address);
        Ok(())
    } else {
        // 配置关闭系统代理：若操作前注册表是开的（无论指向我们还是用户自己的
        // 代理被 import/reset 关闭），必须同步注册表。
        let was_enabled = match win_before {
            Some(s) => s.enabled,
            None => crate::proxy::system_proxy::get_system_proxy()
                .map(|c| c.enabled)
                .unwrap_or(false),
        };
        if !was_enabled {
            return Ok(());
        }
        // 优先还原 journal 里记录的用户原始代理；没有则直接关闭
        let journal_orig = crate::proxy::journal::read_journal(&data_dir)
            .and_then(|j| j.original)
            .filter(|o| o.enabled);
        match journal_orig {
            Some(orig) => crate::proxy::system_proxy::set_system_proxy(
                true,
                &orig.address,
                &orig.bypass_list,
                orig.auto_config_url.as_deref(),
            )?,
            None => crate::proxy::system_proxy::set_system_proxy(false, "", &[], None)?,
        }
        crate::proxy::journal::clear_journal(&data_dir);
        info!("Windows system proxy synced OFF per config intent");
        Ok(())
    }
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
    commit_config_transaction(&app, &state, new_config).await?;
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

/// 重建运行时配置并对运行中的核心生效（热重载，失败回退整进程重启）。
///
/// P0-4：错误必须向上传播——旧实现把 reload 失败吞成 warn 日志后返回成功，
/// 导致「新配置已写盘但 Mihomo 仍用旧值」的假成功。核心未运行时不报错：
/// 文件已重写，下次启动自然加载新配置。
async fn reload_running_core(state: &State<'_, crate::AppState>) -> Result<()> {
    let core_guard = state.core_manager.get();
    if let Some(core) = core_guard.as_ref() {
        core.reload_config().await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
