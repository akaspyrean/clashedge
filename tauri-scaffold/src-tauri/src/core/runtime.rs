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

    // P0-2：全程持有 config_tx，串行整段事务（持久化 → PATCH → 回滚）。
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

    // P0-2：全程持有 config_tx，串行整段事务（见 apply_proxy_mode 注释）。
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

    // 2. 运行中：重写 runtime-config.yaml + PATCH；PATCH 失败回退重启
    let applied = {
        let core_guard = state.core_manager.get();
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
    //
    // 注意：ensure_core_serving 在核心异常时可能触发一次慢速
    // start()/restart()（含就绪轮询，最长 ~10s）。若它在拿到 config_tx
    // 之后执行，会长期占住这把全局锁，导致其余开关（TUN/代理模式/mixin/
    // 托盘）全部排队等待——「很多开关像卡 bug」。此校验只确认核心在服务，
    // 不修改配置，放到事务锁之前执行最安全。
    if enable {
        ensure_core_serving(app).await?;
    }

    // P0-2：全程持有 config_tx，串行整段事务（见 apply_proxy_mode 注释）。
    let _tx = state.config_tx.lock().await;

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

    // P0-1：journal 是崩溃恢复的唯一凭据，必须在改注册表之前持久化成功。
    //   旧顺序："改注册表 → 写 journal（失败仅 warn）" —— 进程在两步之间崩溃
    //   会留下死代理 + 无 journal，下次启动无法恢复。
    //   新顺序（开启路径）：
    //     1) 快照原注册表（before_change，已是开启前状态）
    //     2) 写 journal: original=before_change, owned=true  ← 必须成功
    //     3) 改注册表接管
    //     4) 失败回滚：清 journal + 回滚 config
    //   关闭路径语义变更（用户已授权）：
    //     OFF 不再只是 ProxyEnable=0，而是"退出 ClashEdge 接管"——
    //     读 journal.original 完整还原用户原代理（无则关闭）。这与
    //     sync_windows_side_effects 的 OFF 分支语义一致。
    let data_dir = crate::util::paths::get_app_data_dir(app);
    // journal 写失败只影响**开启**路径——关闭路径不写 journal（读不到原代理时
    // 降级为直接关闭），data_dir 解析失败不应阻塞关闭。
    let journal_err = match (data_dir.as_ref(), enable) {
        (Ok(dir), true) => crate::proxy::journal::write_journal(
            dir,
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
                original: before_change.clone(),
                owned: true,
            },
        )
        .err(),
        (Err(e), true) => {
            // 开启但 data_dir 解析失败 → 无法写 journal，拒绝开启（P0-1 语义）
            Some(Error::Other(format!(
                "Failed to resolve data dir for proxy journal: {}",
                e
            )))
        }
        _ => None,
    };
    if let Some(e) = journal_err {
        // journal 写不进去 → 拒绝开启系统代理（P0-1 语义变更）。
        let mut cfg_mgr = state.config_manager.lock().unwrap();
        let mut cfg = cfg_mgr.get_config();
        cfg.general.system_proxy = old;
        let _ = cfg_mgr.set_config(cfg);
        error!(
            "apply_system_proxy({}) aborted: journal write failed: {}",
            enable, e
        );
        return Err(e);
    }

    // 3. 改注册表（真实接管 / 还原）
    let reg_result = if enable {
        // 开启：接管并删除用户原有 PAC（原值已随 journal.original 保留）
        crate::proxy::system_proxy::set_system_proxy(true, &address, &bypass, None)
    } else {
        // 关闭：只有当我们确实接管了系统代理（journal 存在且 owned=true）才允许
        // 还原/关闭。没有接管凭据 → 不碰注册表——否则会把用户自己（非 ClashEdge）
        // 的系统代理也关掉（例如用户原本用 10.0.0.5:8080，ClashEdge 从未接管过）。
        //
        // 还原语义（与 recover_on_startup 的异常恢复路径保持一致）：
        // - original.enabled = true  → 还原静态代理 address/bypass + 原 PAC；
        // - original.enabled = false → ProxyEnable=0，同时写回 original.auto_config_url
        //   （原 PAC），避免"正常关闭丢 PAC、异常恢复反而保留"的语义不一致；
        // - original = None          → 还原为"无代理"（ProxyEnable=0）。
        let journal = data_dir
            .as_ref()
            .ok()
            .and_then(|d| crate::proxy::journal::read_journal(d));
        match journal.as_ref().filter(|j| j.owned) {
            Some(j) => match &j.original {
                Some(orig) if orig.enabled => crate::proxy::system_proxy::set_system_proxy(
                    true,
                    &orig.address,
                    &orig.bypass_list,
                    orig.auto_config_url.as_deref(),
                ),
                Some(orig) => crate::proxy::system_proxy::set_system_proxy(
                    false,
                    "",
                    &[],
                    orig.auto_config_url.as_deref(),
                ),
                None => crate::proxy::system_proxy::set_system_proxy(false, "", &[], None),
            },
            // 未接管 → 不碰 Windows Registry（保持用户自己的代理不变）
            None => Ok(()),
        }
    };
    if let Err(e) = reg_result {
        // 4. 回滚：注册表改失败 → 还原 config；开启路径下还要清掉刚写的 journal
        //    （否则它会指向一个未生效的接管，下次启动可能误恢复）。
        let mut cfg_mgr = state.config_manager.lock().unwrap();
        let mut cfg = cfg_mgr.get_config();
        cfg.general.system_proxy = old;
        let _ = cfg_mgr.set_config(cfg);
        if enable {
            if let Ok(dir) = &data_dir {
                crate::proxy::journal::clear_journal(dir);
            }
        }
        error!("apply_system_proxy({}) failed, rolled back: {}", enable, e);
        return Err(e);
    }

    // 5. 收尾 journal：
    //    - 接管成功 → journal 已在步骤 2 写好（owned=true）
    //    - 关闭成功 → journal.original 已被还原为用户原状态，journal 失去意义，删除
    if !enable {
        if let Ok(dir) = &data_dir {
            crate::proxy::journal::clear_journal(dir);
        }
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
///
/// 历史遗留的 stop_core 命令自行操作 ConfigManager（直接改 system_proxy 并
/// 吞掉 set_config 错误），绕过 config_tx 事务，与 apply_* 编排层形成两套路径。
/// 本函数是唯一入口。
///
/// 执行顺序（网络安全优先）：先退出系统代理接管，再停止核心。
/// 旧实现先停核心再关系统代理——若关系统代理的 set_config 失败，会留下
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

    // P0-2：全程持有 config_tx，串行整段事务（见 apply_proxy_mode 注释）。
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
