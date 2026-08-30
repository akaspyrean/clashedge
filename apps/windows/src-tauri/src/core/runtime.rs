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

/// mixed-port TCP 探测超时
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

/// 确认 mihomo 正在运行且 mixed-port 真实可连接。
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

    // 已运行且端口可连 → 直接通过；否则自动启动/重启一次。
    // start()/restart() 内部含就绪探测与 bind 冲突检测，失败会返回 Err。
    let ensured = {
        let guard = state.core_manager.get();
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
/// 系统代理开启/恢复失败后，把配置意图落回 Windows 实际状态（false）
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

    // 全程持有 config_tx，串行整段事务（持久化 → PATCH → 回滚）。
    // 与 commit_config_transaction / apply_tun / apply_system_proxy /
    // activate_profile 互斥，避免并发入口交错覆盖。
    let _tx = state.config_tx.lock().await;

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
        let core_guard = state.core_manager.get();
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

    // 3. 失败回滚。回滚持久化失败不得静默吞掉——否则 config.yaml 停在
    //    新值而运行时是旧值，违反五态一致。返回合并错误让调用方与用户感知。
    if let Err(e) = applied {
        let mut cfg_mgr = state.config_manager.lock().unwrap();
        let mut cfg = cfg_mgr.get_config();
        cfg.general.proxy_mode = old_mode.clone();
        let rb = cfg_mgr.set_config(cfg);
        error!("apply_proxy_mode({}) failed, rolled back: {}", mode, e);
        return Err(match rb {
            Ok(()) => e,
            Err(rb_err) => Error::Other(format!(
                "apply_proxy_mode({}) failed ({}), and rollback also failed: {}",
                mode, e, rb_err
            )),
        });
    }

    info!("Proxy mode set to {}", mode);
    refresh_tray(app).await?;
    let _ = app.emit("proxy-mode-changed", serde_json::json!({ "mode": mode }));
    Ok(())
}

/// 应用 TUN 开关：持久化 → 同步运行时 → PATCH 运行中核心 → 确认实际状态。
///
/// 「确认实际结果」是本函数的核心：PATCH /configs 返回 200 不代表 mihomo 真正
/// 接受并运行了目标 TUN 状态（可能静默跳过非法字段 / 内核未能建立网卡）。因此
/// PATCH 后必须回读运行中核心的 `tun.enable`，与目标值比对。
///
/// 流程（对应验证原则）：
/// 1. 持久化新 AppConfig；
/// 2. 重写 runtime-config.yaml；
/// 3. PATCH 运行中核心；PATCH 失败或回读非目标值 → 回退整进程重启；
/// 4. 重启后再次回读确认；仍非目标值 → 视为失败；
/// 5. 失败则完整回滚：恢复旧 AppConfig → 重写旧 runtime-config → 尽力恢复
///    Mihomo 到旧 TUN 状态（必要时重启）→ 返回错误，不假装成功。
///
/// 全程持有 `config_tx` 串行整段事务（见 apply_proxy_mode 注释）。
pub async fn apply_tun(app: &AppHandle, enable: bool) -> Result<()> {
    let state = app.state::<crate::AppState>();

    // 0. 权限预检：开启 TUN 需要管理员权限（Windows 禁止标准用户创建虚拟网卡/
    //    修改路由）——若不满足，直接给出明确提示，不做任何持久化/半套状态。
    //    关闭（enable=false）不受限制。仅 Windows 生效；其它平台 is_elevated 恒 true。
    if enable && !crate::util::elevation::is_elevated() {
        return Err(Error::Other(
            "开启 TUN 需要管理员权限：Windows 要求以管理员身份运行才能创建虚拟网卡并接管路由。\
             请以管理员身份重新运行 ClashEdge 后重试。"
                .to_string(),
        ));
    }

    // 全程持有 config_tx，串行整段事务。
    let _tx = state.config_tx.lock().await;

    // 1. 持久化
    let old = {
        let mut cfg_mgr = state.config_manager.lock().unwrap();
        let mut cfg = cfg_mgr.get_config();
        let old = cfg.tun.enable;
        cfg.tun.enable = enable;
        cfg_mgr.set_config(cfg)?;
        old
    };

    // 2. 运行中：重写 runtime-config.yaml + PATCH + 回读确认；PATCH 失败或
    //    状态回读非目标值 → 回退整进程重启；重启后再确认一次。
    let applied = {
        let core_guard = state.core_manager.get();
        match core_guard.as_ref() {
            Some(core) => {
                if let Err(e) = core.regen_runtime_config() {
                    Err(e)
                } else {
                    let live = live_tun_state(core, enable).await;
                    match live {
                        Ok(()) => Ok(()),
                        Err(patch_err) => {
                            warn!(
                                "TUN live apply/confirm failed ({}); falling back to restart",
                                patch_err
                            );
                            // restart 失败也要走下方回滚，不能 `?` 直接返回
                            match core.restart().await {
                                Ok(()) => live_tun_state(core, enable).await,
                                Err(restart_err) => Err(restart_err),
                            }
                        }
                    }
                }
            }
            None => Ok(()), // 核心未运行：只持久化文件，下次启动即生效，无需实时确认
        }
    };

    // 3. 失败则完整回滚：恢复旧 AppConfig → 重写旧 runtime-config → 尽力恢复
    //    Mihomo 到旧 TUN 状态（PATCH/restart）→ 返回错误，不假装成功。
    if let Err(e) = applied {
        rollback_tun(app, old).await;
        error!("apply_tun({}) failed, rolled back: {}", enable, e);
        return Err(e);
    }

    info!("TUN mode {}", if enable { "enabled" } else { "disabled" });
    refresh_tray(app).await?;
    let _ = app.emit("tun-mode-changed", serde_json::json!({ "enable": enable }));
    Ok(())
}

/// 对运行中核心执行一次「PATCH TUN → 回读确认」。返回 Ok 表示 Mihomo 实际状态
/// 已等于目标；返回 Err 表示 PATCH 失败或回读确认失败（调用方回退重启）。
async fn live_tun_state(core: &crate::core::manager::CoreManager, enable: bool) -> Result<()> {
    // 先确认核心 Running（解决"核心确认 Running"步骤）；
    // 非 Running → 直接视为无法确认，交给调用方重启。
    if core.status() != crate::core::manager::CoreStatus::Running {
        return Err(Error::Other(format!(
            "core not running while applying TUN (status: {})",
            core.status()
        )));
    }
    // PATCH；失败直接向上传播（调用方重启）。
    core.apply_tun(enable).await?;
    // 回读实际状态；非目标 → 视为失败（调用方再重启）。
    let actual = core.get_tun_enable().await?;
    if actual == enable {
        Ok(())
    } else {
        Err(Error::Other(format!(
            "TUN state mismatch after apply: configured {} but running {}",
            enable, actual
        )))
    }
}

/// apply_tun 失败后的完整回滚：恢复旧 AppConfig → 重写旧 runtime-config →
/// 尽力把 Mihomo 恢复旧 TUN 状态（PATCH，失败再整进程重启）。
///
/// 回滚本身失败（旧状态恢复不了）不掩盖原始错误——调用方照样返回原始 Err，
/// 绝不假装成功；core 状态按现有机制进入真实状态。
async fn rollback_tun(app: &AppHandle, old_enable: bool) {
    let state = app.state::<crate::AppState>();

    // 1/2. 恢复旧 AppConfig（内存 + 落盘）并重写旧 runtime-config。
    let restore_ok = {
        let mut cfg_mgr = state.config_manager.lock().unwrap();
        let mut cfg = cfg_mgr.get_config();
        cfg.tun.enable = old_enable;
        match cfg_mgr.set_config(cfg) {
            Ok(()) => {
                if let Some(core) = state.core_manager.get() {
                    core.regen_runtime_config()
                } else {
                    Ok(())
                }
            }
            Err(e) => {
                error!("TUN rollback failed to restore AppConfig: {}", e);
                Err(e)
            }
        }
    };
    if let Err(e) = restore_ok {
        error!(
            "TUN rollback could not restore config/runtime-config: {}",
            e
        );
        return;
    }

    // 3. 尽力把运行中 Mihomo 恢复到旧 TUN 状态；PATCH 失败则整进程重启。
    let core_guard = state.core_manager.get();
    if let Some(core) = core_guard.as_ref() {
        if let Err(e) = live_tun_state(core, old_enable).await {
            warn!(
                "TUN rollback live restore to old state {} failed ({}); restarting",
                old_enable, e
            );
            let _ = core.restart().await;
        }
    }
}

/// 应用系统代理：持久化用户意图 → 写 Windows 注册表（真实生效）→ 失败回滚。
pub async fn apply_system_proxy(app: &AppHandle, enable: bool) -> Result<()> {
    let state = app.state::<crate::AppState>();

    // 开启前必须确认 Core Running 且 mixed-port 实际 TCP 可连接；
    // 不满足时先尝试自动启动核心，仍失败则拒绝开启并返回明确错误——
    // 绝不能让 Windows 指向无人监听的代理端口。此校验在任何持久化之前，
    // 失败时不留下任何半套状态。
    //
    // 注意：ensure_core_serving 在核心异常时可能触发一次慢速
    // start()/restart()（含就绪轮询，最长 ~10s）。若它在拿到 config_tx
    // 之后执行，会长期占住这把全局锁，导致其余开关（TUN/代理模式/mixin/
    // 托盘）全部排队等待——「很多开关像卡 bug」。此校验只确认核心在服务，
    // 不修改配置，放到事务锁之前执行最安全。
    if enable {
        ensure_core_serving(app).await?;
    }

    // 全程持有 config_tx，串行整段事务（见 apply_proxy_mode 注释）。
    let _tx = state.config_tx.lock().await;
    let data_dir = crate::util::paths::get_app_data_dir(app)?;
    let mixed_port = state
        .config_manager
        .lock()
        .unwrap()
        .get_config()
        .general
        .mixed_port;

    // 系统代理开启前密钥兜底：若当前配置仍是占位/空/旧遗留密钥，立即轮换。
    // 系统代理开启后，本机所有流量（含局域网可到达路径）都可能触达本地控制器，
    // 已知默认密钥意味着控制器可被未授权接管——必须先轮换再继续。
    // 轮换复用 ensure_secure_secret 逻辑（经 set_config 落盘生效）。
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

    // 2. 所有接管/释放均走 proxy journal 的统一 ownership helper。
    //    释放失败时 helper 保留 journal；调用方回滚配置并保持 Mihomo 运行。
    let reg_result = if enable {
        crate::proxy::journal::acquire_system_proxy(&data_dir, mixed_port)
    } else {
        crate::proxy::journal::release_owned_proxy(&data_dir, mixed_port).map(|_| ())
    };
    if let Err(e) = reg_result {
        // 3. 回滚配置意图。registry/journal 的安全回滚由统一 helper 负责。
        //    回滚持久化失败不得静默吞掉（五态一致）。
        let mut cfg_mgr = state.config_manager.lock().unwrap();
        let mut cfg = cfg_mgr.get_config();
        cfg.general.system_proxy = old;
        let rb = cfg_mgr.set_config(cfg);
        error!("apply_system_proxy({}) failed, rolled back: {}", enable, e);
        return Err(match rb {
            Ok(()) => e,
            Err(rb_err) => Error::Other(format!(
                "apply_system_proxy({}) failed ({}), and rollback also failed: {}",
                enable, e, rb_err
            )),
        });
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

/// 停止核心并同步系统代理（统一编排入口）。
/// 所有停止核心的调用都必须经由本函数，避免绕过 config_tx 事务或
/// 与 apply_* 编排层形成两套路径。
///
/// 执行顺序（网络安全优先）：先退出系统代理接管，再停止核心。
/// 若先停核心再关系统代理，关系统代理的 set_config 一旦失败，会留下
/// "Windows 代理仍指向已死的 127.0.0.1:7890" 的断网状态。反过来：即使
/// 退系统代理失败，也不停止核心，用户至少保持可上网。
pub async fn stop_core_and_sync_proxy(app: &AppHandle) -> Result<()> {
    // 1) 先退出系统代理接管（config/registry/journal/事件统一事务）。
    //    失败则直接返回，不停止核心——宁可保持核心运行也不掐断用户网络。
    apply_system_proxy(app, false).await?;

    // 2) 再停止核心。
    {
        let state = app.state::<crate::AppState>();
        let core_guard = state.core_manager.get();
        if let Some(core) = core_guard.as_ref() {
            core.stop().await?;
        }
    }

    // apply_system_proxy(false) 末尾的 refresh_tray 发生在核心停止前，此刻
    // 状态仍是 running；需在核心停止后再刷新一次托盘（运行态图标/代理组）。
    refresh_tray(app).await?;
    Ok(())
}

/// 激活 Profile：校验名称合法且文件存在 → 持久化激活名 → 重新生成运行时配置 →
/// 热重载运行中的核心 → 失败回滚。空内容的 Profile 不阻塞：build_runtime_config
/// 会回退到内置模板。
///
/// 持有 `config_tx` 串行整段事务；需要在一个会话内做额外文件/配置变更的调用方
/// 应先用 `activate_profile_locked`（自行先持有 `config_tx`），避免嵌套取锁死锁。
pub async fn activate_profile(app: &AppHandle, name: &str) -> Result<()> {
    let state = app.state::<crate::AppState>();

    // 全程持有 config_tx，串行整段事务（见 apply_proxy_mode 注释）。
    let _tx = state.config_tx.lock().await;

    activate_profile_locked(app, name).await
}

/// 激活 Profile 的事务主体（调用方必须已持有 `config_tx`）。
///
/// 语义与 `activate_profile` 相同，但不取 `config_tx`——供
/// rename/update/refresh 等在一个事务里先做文件变更、再激活的调用方使用，
/// 避免"文件 rename 在事务锁外、activate 才取锁"的交错窗口。
pub(crate) async fn activate_profile_locked(app: &AppHandle, name: &str) -> Result<()> {
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
        let core_guard = state.core_manager.get();
        match core_guard.as_ref() {
            Some(core) => core.restart().await,
            None => Ok(()),
        }
    };

    // 3. 失败回滚。回滚持久化失败不得静默吞掉（五态一致）。
    if let Err(e) = applied {
        let mut cfg_mgr = state.config_manager.lock().unwrap();
        let mut cfg = cfg_mgr.get_config();
        cfg.general.profile = old.clone();
        let rb = cfg_mgr.set_config(cfg);
        error!("activate_profile({}) failed, rolled back: {}", safe, e);
        return Err(match rb {
            Ok(()) => e,
            Err(rb_err) => Error::Other(format!(
                "activate_profile({}) failed ({}), and rollback also failed: {}",
                safe, e, rb_err
            )),
        });
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
        let core_guard = state.core_manager.get();
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
