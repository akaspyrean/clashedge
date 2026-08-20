// src-tauri/src/commands/config.rs
//! 配置命令：获取/更新/重置/导入/导出配置
//!
//! 更新/重置/导入都会改变托盘菜单展示的配置项（模式/系统代理/TUN/混合/
//! 激活 Profile/语言），因此命令完成后再刷新托盘菜单勾选态与文案。

use crate::util::error::Result;
use tauri::{command, AppHandle, State};

#[command]
pub async fn get_config(state: State<'_, crate::AppState>) -> Result<serde_json::Value> {
    let config_guard = state.config_manager.lock().unwrap();
    Ok(serde_json::to_value(config_guard.get_config())?)
}

#[command]
pub async fn update_config(
    app: AppHandle,
    state: State<'_, crate::AppState>,
    config: serde_json::Value,
) -> Result<()> {
    {
        let mut config_guard = state.config_manager.lock().unwrap();
        config_guard.update_config(config)?;
    }
    crate::core::runtime::refresh_tray(&app).await
}

#[command]
pub async fn reset_config(app: AppHandle, state: State<'_, crate::AppState>) -> Result<()> {
    {
        let mut config_guard = state.config_manager.lock().unwrap();
        config_guard.reset_config()?;
    }
    // 重置后按新配置重载核心（否则界面变了、运行中仍是旧配置）。
    reload_running_core(&state).await;
    crate::core::runtime::refresh_tray(&app).await
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
    let mut runtime = crate::core::config::build_runtime_config(&config, profile_content.as_deref())?;
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

/// 从 YAML 导入配置并使其生效：解析 → 校验 → 落盘 → 重建运行时 → 重载核心。
#[command]
pub async fn import_config(
    app: AppHandle,
    state: State<'_, crate::AppState>,
    yaml: String,
) -> Result<()> {
    {
        let mut config_guard = state.config_manager.lock().unwrap();
        config_guard.import_config(yaml)?;
    }
    // 导入的参数立即生效（重建 runtime-config + 重载运行中的核心）
    reload_running_core(&state).await;
    crate::core::runtime::refresh_tray(&app).await
}

/// 重建运行时配置并热重载运行中的核心；核心未运行时仅重写文件
/// （下次启动自然加载新配置）。reload_config 内部会先重写 runtime-config.yaml。
async fn reload_running_core(state: &State<'_, crate::AppState>) {
    let core_guard = state.core_manager.lock().await;
    if let Some(core) = core_guard.as_ref() {
        if let Err(e) = core.reload_config().await {
            tracing::warn!("Failed to reload core after config change: {}", e);
        }
    }
    // guard 在函数返回时释放
}
