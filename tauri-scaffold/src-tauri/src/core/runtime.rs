//! 统一编排层：apply_proxy_mode / apply_tun / apply_system_proxy
//!
//! 前端命令（commands/proxy.rs）与托盘事件（tray/events.rs）都调用这里，
//! 消除两套重复的"改配置 + 动核心"逻辑，确保
//! 「界面状态 = 应用状态 = Mihomo 实际状态 = Windows 实际状态」。
//!
//! 每个入口统一遵循：
//! 1. 校验输入（proxy mode 必须是 mihomo 合法值）；
//! 2. 持久化到共享配置 Arc（内存 + 原子落盘，单一数据源）；
//! 3. 重新生成 runtime-config.yaml（下次启动/重载即生效）；
//! 4. 实时应用（运行中：REST PATCH / configs；失败回退重启或回滚）；
//! 5. 失败则回滚配置；
//! 6. 推送事件给前端，并刷新托盘菜单（勾选状态来自真实状态）。
//!
//! 锁纪律：`config_manager` 是 std Mutex（不得跨 `.await` 持有），
//! `core_manager` 是 tokio Mutex（可跨 `.await` 持有）。代码中任何一段
//! 都不同时在两个锁上等待。

use tauri::{AppHandle, Emitter, Manager};
use tracing::{error, info, warn};

use crate::tray::builder::{ProxyGroupInfo, ProxySubgroupInfo};
use crate::util::error::{Error, Result};
use crate::util::paths::sanitize_profile_name;

/// mihomo 合法代理模式（官方模板仅这三值；script 是 Clash Premium 遗留）
const VALID_MODES: &[&str] = &["rule", "global", "direct"];

/// P0-2：mixed-port TCP 探测超时
const PORT_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// mixed-port 是否真实可连接（TCP 握手成功）
pub(crate) async fn port_alive(port: u16) -> bool {
    matches!(
        tokio::time::timeout(
            PORT_PROBE_TIMEOUT,
            tokio::net::TcpStream::connect(("127.0.0.1", port))
        )
        .await,
        Ok(Ok(_))
    )
}

/// P0-2：确认 mihomo 正在运行且 mixed-port 真实可连接。
///
/// 开启系统代理前必须调用——绝不能让 Windows 指向无人监听的代理端口（死代理）。
/// 核心未运行或端口不可连时，按方案优先级先尝试自动启动一次核心；
/// 启动失败或启动后端口仍不可连，返回明确错误（调用方拒绝开启系统代理）。
pub(crate) async fn ensure_core_serving(app: &AppHandle) -> Result<()> {
    let state = app.state::<crate::AppState>();
    let port = {
        state
            .config_manager
            .lock()
            .unwrap()
            .get_config()
            .general
            .mixed_port
    };

    // 已运行且端口可连 → 直接通过；否则自动启动/重启一次（P0-2 方案 1）。
    // start()/restart() 内部含就绪探测与 bind 冲突检测，失败会返回 Err。
    let ensured = {
        let guard = state.core_manager.lock().await;
        match guard.as_ref() {
            Some(core) if core.status() == crate::core::manager::CoreStatus::Running => {
                if port_alive(port).await {
                    Ok(())
                } else {
                    warn!(
                        "Core status=running but mixed-port {} not accepting; restarting",
                        port
                    );
                    core.restart().await
                }
            }
            Some(core) => {
                warn!(
                    "System proxy requested but core not running ({}); starting",
                    core.status()
                );
                core.start().await
            }
            None => Err(Error::Other("core manager unavailable".to_string())),
        }
    };
    if let Err(e) = ensured {
        return Err(Error::Other(format!(
            "拒绝开启系统代理：内核未运行且自动启动失败（{}）",
            e
        )));
    }

    // 终局校验：端口必须真实可连接，否则视为失败
    if !port_alive(port).await {
        return Err(Error::Other(format!(
            "拒绝开启系统代理：mihomo 运行中但端口 {} 无监听",
            port
        )));
    }
    Ok(())
}
/// 系统代理绕过列表（ProxyOverride）：本机/局域网直连。
/// 必须显式列出 127.0.0.1 / localhost / *.tauri.localhost：
/// - 系统代理开启时，WebView2 的前端资源与 IPC 走 tauri.localhost / 127.0.0.1；
/// - mihomo 外部控制器在 127.0.0.1:9090，后端用 reqwest 直连；
/// - 仅靠 `<local>`（对应空主机名/局域网）不会覆盖字面量 IP 127.0.0.1 与
///   tauri.localhost 域名，导致这些回环请求被错误地代理到 mihomo 的 7890，
///   在内核未就绪/重启时返回 ERR_CONNECTION_REFUSED（BUG1 根因之一）。
///
/// pub(crate)：CoreSupervisor 在崩溃自愈恢复系统代理时复用同一份绕过列表，
/// 保证恢复值与正常开启路径完全一致。
pub(crate) fn default_bypass() -> Vec<String> {
    vec![
        "<local>".to_string(),
        "127.0.0.1".to_string(),
        "localhost".to_string(),
        "*.tauri.localhost".to_string(),
    ]
}

/// P0-3：系统代理开启/恢复失败后，把配置意图落回 Windows 实际状态（false）
/// 并推送事件——不允许 UI 在注册表实际关闭时继续把开关显示为 ON。
pub(crate) async fn mark_system_proxy_failed(app: &AppHandle, reason: &str) {
    let state = app.state::<crate::AppState>();
    {
        let mut cfg_mgr = state.config_manager.lock().unwrap();
        let mut cfg = cfg_mgr.get_config();
        if cfg.general.system_proxy {
            cfg.general.system_proxy = false;
            if let Err(e) = cfg_mgr.set_config(cfg) {
                error!(
                    "Failed to persist system_proxy=false after failure ({}): {}",
                    reason, e
                );
                return;
            }
        }
    }
    let _ = app.emit(
        "system-proxy-changed",
        serde_json::json!({ "enable": false, "error": reason }),
    );
}

/// 应用代理模式：校验 → 持久化 → 同步运行时 → PATCH 运行中核心 → 失败回滚。
pub async fn apply_proxy_mode(app: &AppHandle, mode: &str) -> Result<()> {
    if !VALID_MODES.contains(&mode) {
        return Err(Error::InvalidArgument(format!(
            "invalid proxy mode '{}' (must be rule/global/direct)",
            mode
        )));
    }
    let state = app.state::<crate::AppState>();

    // 1. 持久化（内存 + 原子落盘）
    let old_mode = {
        let mut cfg_mgr = state.config_manager.lock().unwrap();
        let mut cfg = cfg_mgr.get_config();
        let old = cfg.general.proxy_mode.clone();
        cfg.general.proxy_mode = mode.to_string();
        cfg_mgr.set_config(cfg)?;
        old
    };

    // 2. 运行中：先重写 runtime-config.yaml（下次重启保持新值），再 PATCH 实时生效
    let applied = {
        let core_guard = state.core_manager.lock().await;
        match core_guard.as_ref() {
            Some(core) => {
                if let Err(e) = core.regen_runtime_config() {
                    Err(e)
                } else {
                    core.set_proxy_mode(mode.to_string()).await
                }
            }
            None => Ok(()),
        }
    };

    // 3. 失败回滚
    if let Err(e) = applied {
        let mut cfg_mgr = state.config_manager.lock().unwrap();
        let mut cfg = cfg_mgr.get_config();
        cfg.general.proxy_mode = old_mode;
        let _ = cfg_mgr.set_config(cfg);
        error!("apply_proxy_mode({}) failed, rolled back: {}", mode, e);
        return Err(e);
    }

    info!("Proxy mode set to {}", mode);
    refresh_tray(app).await?;
    let _ = app.emit("proxy-mode-changed", serde_json::json!({ "mode": mode }));
    Ok(())
}

/// 应用 TUN 开关：持久化 → 同步运行时 → PATCH 运行中核心；
/// PATCH 失败（TUN 常需重启才生效）回退整进程重启；重启也失败则回滚。
pub async fn apply_tun(app: &AppHandle, enable: bool) -> Result<()> {
    let state = app.state::<crate::AppState>();

    // 1. 持久化
    let old = {
        let mut cfg_mgr = state.config_manager.lock().unwrap();
        let mut cfg = cfg_mgr.get_config();
        let old = cfg.tun.enable;
        cfg.tun.enable = enable;
        cfg_mgr.set_config(cfg)?;
        old
    };

    // 2. 运行中：重写 runtime-config.yaml + PATCH；PATCH 失败回退重启
    let applied = {
        let core_guard = state.core_manager.lock().await;
        match core_guard.as_ref() {
            Some(core) => {
                if let Err(e) = core.regen_runtime_config() {
                    Err(e)
                } else {
                    match core.apply_tun(enable).await {
                        Ok(()) => Ok(()),
                        Err(patch_err) => {
                            warn!(
                                "TUN live apply failed ({}); falling back to restart",
                                patch_err
                            );
                            core.restart().await
                        }
                    }
                }
            }
            None => Ok(()),
        }
    };

    // 3. 失败回滚
    if let Err(e) = applied {
        let mut cfg_mgr = state.config_manager.lock().unwrap();
        let mut cfg = cfg_mgr.get_config();
        cfg.tun.enable = old;
        let _ = cfg_mgr.set_config(cfg);
        error!("apply_tun({}) failed, rolled back: {}", enable, e);
        return Err(e);
    }

    info!("TUN mode {}", if enable { "enabled" } else { "disabled" });
    refresh_tray(app).await?;
    let _ = app.emit("tun-mode-changed", serde_json::json!({ "enable": enable }));
    Ok(())
}

/// 应用系统代理：持久化用户意图 → 写 Windows 注册表（真实生效）→ 失败回滚。
pub async fn apply_system_proxy(app: &AppHandle, enable: bool) -> Result<()> {
    let state = app.state::<crate::AppState>();

    // P0-2：开启前必须确认 Core Running 且 mixed-port 实际 TCP 可连接；
    // 不满足时先尝试自动启动核心，仍失败则拒绝开启并返回明确错误——
    // 绝不能让 Windows 指向无人监听的代理端口。此校验在任何持久化之前，
    // 失败时不留下任何半套状态。
    if enable {
        ensure_core_serving(app).await?;
    }

    // C9 系统代理开启前密钥兜底：若当前配置仍是占位/空/旧遗留密钥，立即轮换。
    // 系统代理开启后，本机所有流量（含局域网可到达路径）都可能触达本地控制器，
    // 已知默认密钥意味着控制器可被未授权接管——必须先轮换再继续。
    // 轮换复用 H1 的 ensure_secure_secret 逻辑（经 set_config 落盘生效）。
    if enable {
        let mut cfg_mgr = state.config_manager.lock().unwrap();
        let cfg = cfg_mgr.get_config();
        if crate::config::model::needs_secret_rotation(&cfg.proxy.secret) {
            info!("Rotating controller secret before enabling system proxy");
            cfg_mgr.set_config(cfg)?;
        }
    }

    // 1. 持久化用户意图（与 allow-lan 分离的独立状态）
    let old = {
        let mut cfg_mgr = state.config_manager.lock().unwrap();
        let mut cfg = cfg_mgr.get_config();
        let old = cfg.general.system_proxy;
        cfg.general.system_proxy = enable;
        cfg_mgr.set_config(cfg)?;
        old
    };

    // 2. 写注册表（真实生效）。写之前快照当前 Windows 代理状态，
    //    作为 Recovery Journal 的"用户原始状态"记录。
    let address = {
        let cfg = state.config_manager.lock().unwrap().get_config();
        format!("127.0.0.1:{}", cfg.general.mixed_port)
    };
    let before_change = crate::proxy::system_proxy::get_system_proxy().ok();
    let bypass = default_bypass();
    if let Err(e) = crate::proxy::system_proxy::set_system_proxy(enable, &address, &bypass) {
        // 3. 回滚
        let mut cfg_mgr = state.config_manager.lock().unwrap();
        let mut cfg = cfg_mgr.get_config();
        cfg.general.system_proxy = old;
        let _ = cfg_mgr.set_config(cfg);
        error!("apply_system_proxy({}) failed, rolled back: {}", enable, e);
        return Err(e);
    }

    // 4. P1-8 Recovery Journal：
    //    - 接管成功 → 记录"接管前"的原始代理状态（断电/强杀后的启动自愈依据）；
    //    - 主动关闭成功 → 清除 journal（干净关闭无需恢复）。
    match crate::util::paths::get_app_data_dir(app) {
        Ok(data_dir) => {
            if enable {
                crate::proxy::journal::write_journal(
                    &data_dir,
                    &crate::proxy::journal::ProxyJournal {
                        session_id: format!(
                            "{:016x}{:016x}",
                            rand::random::<u64>(),
                            rand::random::<u64>()
                        ),
                        pid: std::process::id(),
                        mixed_port: address
                            .rsplit(':')
                            .next()
                            .and_then(|p| p.parse().ok())
                            .unwrap_or(0),
                        original: before_change,
                        owned: true,
                    },
                );
            } else {
                crate::proxy::journal::clear_journal(&data_dir);
            }
        }
        Err(e) => warn!("Failed to resolve data dir for proxy journal: {}", e),
    }

    info!(
        "System proxy {}",
        if enable { "enabled" } else { "disabled" }
    );
    refresh_tray(app).await?;
    let _ = app.emit(
        "system-proxy-changed",
        serde_json::json!({ "enable": enable }),
    );
    Ok(())
}

/// 激活 Profile：校验名称合法且文件存在 → 持久化激活名 → 重新生成运行时配置 →
/// 热重载运行中的核心 → 失败回滚。空内容的 Profile 不阻塞：build_runtime_config
/// 会回退到内置模板。
pub async fn activate_profile(app: &AppHandle, name: &str) -> Result<()> {
    let state = app.state::<crate::AppState>();

    // 0. 校验名称（sanitize 防路径穿越）。
    //    "DIRECT" 是内置预设（无对应文件，build_runtime_config 用内置骨架），
    //    其余名字必须存在对应文件。
    let safe = sanitize_profile_name(name)?;
    let is_builtin = safe.eq_ignore_ascii_case("DIRECT");
    let data_dir = crate::util::paths::get_app_data_dir(app)?;
    let path = data_dir.join("profiles").join(format!("{}.yaml", safe));
    if !path.exists() && !is_builtin {
        return Err(Error::NotFound(format!("Profile '{}' not found", safe)));
    }

    // 1. 持久化激活名（内存 + 原子落盘）
    let old = {
        let mut cfg_mgr = state.config_manager.lock().unwrap();
        let mut cfg = cfg_mgr.get_config();
        let old = cfg.general.profile.clone();
        cfg.general.profile = safe.clone();
        cfg_mgr.set_config(cfg)?;
        old
    };

    // 2. 整进程重启：Profile 切换会改变代理列表，mihomo REST 热重载对 proxy
    //    定义列表不可靠（返回 200 但节点可能未注入），改为重启保证生效。
    let applied = {
        let core_guard = state.core_manager.lock().await;
        match core_guard.as_ref() {
            Some(core) => core.restart().await,
            None => Ok(()),
        }
    };

    // 3. 失败回滚
    if let Err(e) = applied {
        let mut cfg_mgr = state.config_manager.lock().unwrap();
        let mut cfg = cfg_mgr.get_config();
        cfg.general.profile = old;
        let _ = cfg_mgr.set_config(cfg);
        error!("activate_profile({}) failed, rolled back: {}", safe, e);
        return Err(e);
    }

    info!("Profile activated: {}", safe);
    refresh_tray(app).await?;
    let _ = app.emit("profile-activated", serde_json::json!({ "profile": safe }));
    Ok(())
}

/// 刷新托盘菜单：勾选状态来自当前共享配置 + 核心状态（真实状态，而非旧值）。
/// 代理组子菜单在核心运行且可达时填充真实组/节点。
pub(crate) async fn refresh_tray(app: &AppHandle) -> Result<()> {
    let state = app.state::<crate::AppState>();

    let config = { state.config_manager.lock().unwrap().get_config() };
    let i18n = crate::i18n::loader::I18n::new(&config.locale);

    let (core_status, proxies) = {
        let core_guard = state.core_manager.lock().await;
        let status = core_guard.as_ref().map(|c| c.status()).unwrap_or_default();
        let groups = match core_guard.as_ref() {
            Some(c) => c.get_proxy_groups().await.unwrap_or_default(),
            None => Vec::new(),
        };
        (status, core_groups_to_tray(&groups))
    };

    let tray_guard = state.tray.lock().unwrap();
    if let Some(tray) = tray_guard.as_ref() {
        let handle = tray.app_handle().clone();
        crate::tray::builder::update_tray_menu(
            &handle,
            tray,
            &core_status,
            &proxies,
            &config,
            &i18n,
        )?;
    }
    Ok(())
}

/// 把核心返回的代理组 JSON（{name,type,now,all}）转成托盘子菜单结构：
/// 每个组成为一项，其 `all` 节点作为子项，当前选中（now）勾选。
fn core_groups_to_tray(groups: &[serde_json::Value]) -> Vec<ProxyGroupInfo> {
    groups
        .iter()
        .filter_map(|g| {
            let name = g.get("name").and_then(|v| v.as_str())?.to_string();
            let now = g
                .get("now")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let all = g
                .get("all")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let subgroups: Vec<ProxySubgroupInfo> = all
                .iter()
                .filter_map(|p| {
                    p.as_str().map(|s| ProxySubgroupInfo {
                        name: s.to_string(),
                        is_selected: s == now,
                    })
                })
                .collect();
            Some(ProxyGroupInfo {
                name,
                is_selected: false, // 组级勾选无意义，子项才带勾选
                subgroups,
            })
        })
        .collect()
}
