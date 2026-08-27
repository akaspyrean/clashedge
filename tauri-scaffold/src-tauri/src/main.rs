// src-tauri/src/main.rs
//! ClashEdge - Tauri 2 入口
//!
//! 架构：
//! - 后端：Rust (Tauri 2 + Tokio)
//! - 前端：Vue 3 + TypeScript + Pinia + Element Plus
//! - 打包：dir target（便携包，无安装器）
//! - Sidecar：clash-edge-core.exe, go-tun2socks.exe, EnableLoopback.exe, wintun.dll
//! - 便携模式：exe 同目录 App/ 存放程序文件，Data/ 存放用户数据
//!   （原生便携检测：App/portable.dat 或 App/clash-edge-core.exe 存在即便携）

// Windows GUI 子系统：否则双击内层 exe 会弹一个黑色控制台窗口（大黑框）。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(clippy::needless_return)]

mod commands;
mod config;
mod core;
mod geodata;
mod i18n;
mod proxy;
mod tray;
mod update;
mod util;

use crate::config::persistence::ConfigManager;
use crate::core::manager::CoreManager;
use crate::tray::builder::build_tray;
use crate::util::logging::init as init_logging;
use crate::util::paths::get_app_data_dir;
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};
use tracing::{error, info, warn};

/// 应用状态 - 需要公开以便 commands 访问
///
/// 注意锁类型：
/// - `core_manager` 初始化后稳定持有（OnceLock），只读 REST 操作并发执行，
///   生命周期操作（start/stop/restart/reload）走内部 lifecycle 互斥锁。
/// - `config_manager` / `tray` 是 std Mutex（同步锁，禁止跨 `.await` 持有），
///   只保护极短的临界区（读快照、set_config 落盘）。跨 `.await` 的运行时
///   操作（reload / restart / Windows 副作用）会释放该锁，因此 `config_manager`
///   **不能**串行整个配置事务——两个并发事务会交错：A 写 V2 后 await reload，
///   B 读到 V2 写 V3，A 失败回滚到 V1 覆盖 B 的 V3。
/// - `config_tx` 是 tokio Mutex，可跨 `.await` 持有。所有改变系统运行态的入口
///   （update_config / update_config_fields / reset_config / import_config /
///   apply_proxy_mode / apply_tun / apply_system_proxy / activate_profile /
///   tray config_mixin）必须在做事之前先 `config_tx.lock().await` 并持有到
///   事务结束，保证「UI = Config = runtime-config = Mihomo = Windows」在并发
///   入口下也严格成立。锁的是 `()` —— 纯串行作用，不承载任何数据。
pub struct AppState {
    pub core_manager: std::sync::OnceLock<crate::core::manager::CoreManager>,
    pub config_manager: std::sync::Mutex<ConfigManager>,
    /// 配置/运行态事务串行锁：跨 `.await` 持有，串行所有改 Config + Mihomo +
    /// Windows 的入口。见上文结构注释。P0-2。
    pub config_tx: tokio::sync::Mutex<()>,
    pub tray: std::sync::Mutex<Option<tauri::tray::TrayIcon>>,
    /// 日志流任务句柄（前端日志页启用/停止；std Mutex，abort 为同步调用）
    pub log_stream: std::sync::Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    /// P1-7：最近一次 mihomo 子进程 PID 缓存（CoreSupervisor 在每次成功 spawn
    /// 后更新、stop 时清零）。退出清理在 core_manager 锁被 async 任务占用时
    /// 用它做按 PID 精确清杀，取代旧的「按进程名 taskkill」——后者会误杀
    /// 用户自己在跑的其他 mihomo 实例。0 = 本会话从未启动过子进程。
    pub core_pid_cache: std::sync::atomic::AtomicU32,
    /// P0-6：本会话最近一次通过 minisign 验签的更新清单（附验签时刻）。
    /// `download_update` 只允许下载这份清单指向的包——WebView 传入的
    /// version/url/hash 不参与任何决策。带 TTL：距检查超过
    /// `VERIFIED_UPDATE_TTL` 后缓存失效，必须重新 check_update，
    /// 防止用陈旧清单下载已被撤回/替换的版本。
    pub verified_update:
        std::sync::Mutex<Option<(crate::update::UpdateManifest, std::time::Instant)>>,
}

impl AppState {
    fn new() -> Self {
        Self {
            core_manager: std::sync::OnceLock::new(),
            config_manager: std::sync::Mutex::new(ConfigManager::new()),
            config_tx: tokio::sync::Mutex::new(()),
            tray: std::sync::Mutex::new(None),
            log_stream: std::sync::Mutex::new(None),
            core_pid_cache: std::sync::atomic::AtomicU32::new(0),
            verified_update: std::sync::Mutex::new(None),
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        // 单实例：重复启动时聚焦已存在的主窗口，而不是再起一份
        // （托盘应用 + 便携包，双实例会导致两个进程同时操作同一份配置）。
        // 静默自启（--clash-edge-autostart）时已有实例在跑，不应弹窗打扰。
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            let autostart = args.iter().any(|a| a == "--clash-edge-autostart");
            if autostart {
                return;
            }
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        // 文件选择保留 dialog；fs / notification / clipboard / opener 无真实调用，
        // 文件读写与打开目录均走受控 Rust command，因此不注册未使用插件。
        .plugin(tauri_plugin_dialog::init())
        // Tauri updater 半成品已移除；当前更新链是已实现的 Portable Updater
        //（minisign manifest → SHA256 → staging → Launcher 事务替换）。
        .plugin(init_logging())
        .manage(AppState::new())
        .setup(|app| {
            info!("ClashEdge starting...");

            // 获取数据目录（便携 Data/ 或 Tauri 默认 app_data_dir）
            let app_handle = app.handle().clone();
            let data_dir = get_app_data_dir(&app_handle)?;
            info!("Data directory: {:?}", data_dir);

            // 便携包复制/移动/改名后，修复开机自启注册表里可能指向旧位置的路径
            if let Err(e) = crate::util::autostart::repair_autostart() {
                warn!("Failed to repair autostart path: {}", e);
            }

            // 初始化配置管理器
            {
                let state = app.state::<AppState>();
                let mut config_mgr = state.config_manager.lock().unwrap();
                config_mgr.init(&data_dir)?;

                // P1-8：异常恢复与手动关闭/正常退出共用同一 ownership helper。
                // ownership 已被用户/其他软件拿走时，不写注册表，并把配置意图落回
                // false，避免核心启动后再次覆盖用户的新代理。
                let port = config_mgr.get_config().general.mixed_port;
                match crate::proxy::journal::recover_on_startup(&data_dir, port) {
                    Ok(crate::proxy::journal::ReleaseOutcome::Restored { message, .. }) => {
                        warn!("Proxy journal recovery: {}", message);
                    }
                    Ok(crate::proxy::journal::ReleaseOutcome::OwnershipLost) => {
                        let mut cfg = config_mgr.get_config();
                        cfg.general.system_proxy = false;
                        config_mgr.set_config(cfg)?;
                        warn!(
                            "Proxy ownership changed after abnormal exit; preserving Windows state and disabling ClashEdge proxy intent"
                        );
                    }
                    Ok(crate::proxy::journal::ReleaseOutcome::NoOwnership) => {}
                    Err(e) => {
                        warn!("Proxy journal recovery deferred; journal kept: {}", e);
                    }
                }
            }

            // 初始化核心管理器（与 ConfigManager 共享同一个配置 Arc，单一数据源）
            {
                let state = app.state::<AppState>();
                let config_handle = { state.config_manager.lock().unwrap().config_handle() };
                let core_mgr = CoreManager::new(app.handle().clone(), config_handle)?;
                state.core_manager.set(core_mgr).ok();
            }

            // 创建系统托盘（内部会把 TrayIcon 存入 AppState.tray）
            build_tray(app.handle())?;

            // 设置窗口行为
            // H2③ WebView 导航锁定：仅放行应用自身 origin，其余一律阻止。
            // 回调返回 true 放行、false 拒绝；拒绝时 WebView 停留在当前页面，
            // 防止被导航到外部站点（钓鱼 / 恶意注入 / 加载外部脚本）。
            // 放行清单：
            //   - tauri://localhost            （Tauri 自定义协议）
            //   - http/https://tauri.localhost（应用自身资源/IPC origin）
            //   - debug 构建额外放行 dev 服务器 http://localhost:1420（tauri.conf.json devUrl）
            //
            // Tauri 2 仅在 WebviewWindowBuilder 上提供 on_navigation（窗口实例与
            // 配置式窗口均无此钩子），因此主窗口改为在 setup 中通过 Builder 创建，
            // 窗口属性与原先 tauri.conf.json app.windows[0] 保持一致。
            let window =
                WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                    .title("ClashEdge")
                    // 默认尺寸 832×554（紧凑基准 756×504 上浮 10%）；
                    // 高于前端窄窗阈值 749，侧栏文字正常展示。
                    .inner_size(832.0, 554.0)
                    .min_inner_size(560.0, 400.0)
                    .background_color(tauri::window::Color(0x10, 0x12, 0x14, 0xff))
                    .decorations(false)
                    .resizable(true)
                    .maximizable(true)
                    .minimizable(true)
                    .closable(false)
                    .center()
                    .visible(false)
                    .on_navigation(|url| {
                        let allowed = match (url.scheme(), url.host_str()) {
                            ("tauri", Some("localhost")) => true,
                            ("http" | "https", Some("tauri.localhost")) => true,
                            #[cfg(debug_assertions)]
                            ("http", Some("localhost")) => url.port() == Some(1420),
                            _ => false,
                        };
                        if !allowed {
                            warn!(
                                "Navigation blocked (outside allowed origins): {}",
                                url.as_str()
                            );
                        }
                        allowed
                    })
                    .build()?;

            // 任务栏/窗口图标：与桌面（exe 内嵌 cat.ico 资源）图标保持一致。
            // Tauri 默认窗口图标可能与 exe 资源图标不一致，这里显式设置。
            #[cfg(target_os = "windows")]
            {
                let bytes = include_bytes!("../icons/cat-256x256.png");
                match image::load_from_memory(bytes) {
                    Ok(img) => {
                        let rgba = img.to_rgba8();
                        let (w, h) = rgba.dimensions();
                        let icon = tauri::image::Image::new_owned(rgba.into_raw(), w, h);
                        let _ = window.set_icon(icon);
                    }
                    Err(e) => warn!("Failed to decode window icon: {}", e),
                }
            }
            #[cfg(target_os = "windows")]
            {
                let _ = window.set_decorations(false);
            }

            // 处理窗口关闭事件（最小化到托盘）
            let app_handle = app.handle().clone();
            window.on_window_event(move |event| {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = app_handle.get_webview_window("main").map(|w| w.hide());
                }
            });

            // 启动核心服务（异步任务内部重新加锁）。
            // 顺序：先启动 mihomo 并确认监听就绪，再启用系统代理。
            // 反过来（先写注册表代理再起内核）会在 mihomo 真正监听 7890 之前
            // 把系统代理指向一个还没人监听的端口，这段时间内所有走代理的请求
            // （含 WebView2 自身的资源/IPC 回环请求）都会得到 ERR_CONNECTION_REFUSED。
            // 开机自启时系统刚登录、网络/磁盘可能未就绪，首次启动失败很常见——
            // 重试数次，避免开机后内核一直不跑。
            {
                let app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let state = app_handle.state::<AppState>();
                    let sys_proxy_intent = state
                        .config_manager
                        .lock()
                        .unwrap()
                        .get_config()
                        .general
                        .system_proxy;

                    const START_RETRIES: u32 = 5;
                    let mut attempt: u32 = 0;
                    let mut started = false;
                    loop {
                        let ok = match state.core_manager.get() {
                            Some(core) => core.start().await.is_ok(),
                            None => false,
                        };
                        if ok {
                            started = true;
                            break;
                        }
                        attempt += 1;
                        if attempt >= START_RETRIES {
                            error!("Failed to start core after {} attempts", attempt);
                            break;
                        }
                        info!(
                            "Core start failed (attempt {}/{}); retrying in 2s",
                            attempt, START_RETRIES
                        );
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    }

                    // 只有内核真正起来之后，才把系统代理打开——避免代理指向死端口。
                    if started && sys_proxy_intent {
                        if let Err(e) =
                            crate::core::runtime::apply_system_proxy(&app_handle, true).await
                        {
                            error!("Failed to restore system proxy: {}", e);
                            // P0-3：恢复失败不得让 UI 继续把 system-proxy 当作 ON；
                            // 配置落回实际状态并推送事件刷新前端。
                            crate::core::runtime::mark_system_proxy_failed(
                                &app_handle,
                                &e.to_string(),
                            )
                            .await;
                        }
                    } else if sys_proxy_intent && !started {
                        // 内核没起来但配置里仍想开系统代理：保持关闭，否则会指向死端口。
                        // P0-3：配置意图同步落回 false，UI 显示真实状态（OFF）。
                        warn!("System proxy stays OFF: core failed to start (config intent=true)");
                        crate::core::runtime::mark_system_proxy_failed(
                            &app_handle,
                            "core failed to start",
                        )
                        .await;
                    }
                });
            }

            // D6：启动时一次性订阅静默刷新——延迟 60s（给核心启动与网络就绪
            // 留时间）后执行一次即结束：无常驻定时器、无循环、不占驻留内存。
            // 仅在用户显式开启"自动更新订阅"时执行（可预期性与隐私：避免
            // 用户只是打开应用、60s 后却静默访问订阅服务器）。
            {
                let app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let enabled = app_handle
                        .state::<crate::AppState>()
                        .config_manager
                        .lock()
                        .unwrap()
                        .get_config()
                        .general
                        .auto_update_subscription;
                    if !enabled {
                        info!("Startup subscription auto-refresh disabled by setting");
                        return;
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                    crate::commands::profiles::auto_refresh_stale_subscriptions(&app_handle).await;
                    info!("Startup subscription auto-refresh finished");
                });
            }

            info!("ClashEdge started successfully");
            Ok(())
        })
        // 注意：必须使用 `crate::commands::...` 完整路径。
        // 裸路径 `core::`/`config::`/`proxy::` 会命中 crate 根部的同名模块
        // （`mod core` 等），导致 E0433：找不到 `__cmd__*` / `__tauri_command_name_*`。
        .invoke_handler(tauri::generate_handler![
            // Core commands
            crate::commands::core::get_status,
            crate::commands::core::start_core,
            crate::commands::core::stop_core,
            crate::commands::core::restart_core,
            crate::commands::core::reload_config,
            // Config commands
            crate::commands::config::get_config,
            crate::commands::config::update_config,
            crate::commands::config::update_config_fields,
            crate::commands::config::reset_config,
            crate::commands::config::export_config,
            crate::commands::config::import_config,
            crate::commands::config::pick_import_file,
            // Connections commands
            crate::commands::connections::get_connections,
            crate::commands::connections::close_all_connections,
            // Proxy commands
            crate::commands::proxy::set_system_proxy,
            crate::commands::proxy::set_tun_mode,
            crate::commands::proxy::set_proxy_mode,
            crate::commands::proxy::test_proxy_latency,
            crate::commands::proxy::get_proxy_groups,
            crate::commands::proxy::select_proxy_group,
            // Logs commands
            crate::commands::logs::start_log_stream,
            crate::commands::logs::stop_log_stream,
            // GeoData commands
            crate::commands::geodata::update_geodata,
            crate::commands::geodata::get_geodata_status,
            crate::commands::geodata::get_geodata_urls,
            crate::commands::geodata::set_geodata_urls,
            // Profile commands
            crate::commands::profiles::list_profiles,
            crate::commands::profiles::create_profile,
            crate::commands::profiles::delete_profile,
            crate::commands::profiles::rename_profile,
            crate::commands::profiles::activate_profile,
            crate::commands::profiles::get_profile_content,
            crate::commands::profiles::update_profile_content,
            crate::commands::profiles::import_profile,
            crate::commands::profiles::import_profile_from_url,
            crate::commands::profiles::update_profile_subscription,
            crate::commands::profiles::export_profile,
            // Tray commands
            crate::commands::tray::get_tray_menu_state,
            crate::commands::tray::update_tray_menu,
            // Utility commands
            crate::commands::util::open_data_dir,
            crate::commands::util::open_logs_dir,
            crate::commands::util::get_app_version,
            crate::commands::util::is_autostart,
            crate::commands::util::get_autostart,
            crate::commands::util::set_autostart,
            crate::commands::util::get_supported_locales,
            crate::commands::util::set_locale,
            crate::commands::util::get_i18n_messages,
            // Portable Updater commands
            crate::commands::update::check_update,
            crate::commands::update::download_update,
            crate::commands::update::get_staged_update,
            crate::commands::update::discard_staged_update,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    // 挂接 RunEvent 生命周期：退出时先安全释放系统代理，再停止 mihomo，
    // 否则 mihomo 变成孤儿进程继续代理、系统代理残留指向死端口，全网断开。
    app.run(|app_handle, event| {
        if let tauri::RunEvent::Exit = event {
            cleanup_on_exit(app_handle);
        }
    });
}

/// 退出清理严格顺序：确认 ownership → 精确恢复 → 复读确认 → 停止 Mihomo →
/// 清 journal。恢复/确认失败立即返回，核心与 journal 都保留，避免制造死代理。
fn cleanup_on_exit(app_handle: &tauri::AppHandle) {
    let state = app_handle.state::<AppState>();
    let mixed_port = state
        .config_manager
        .lock()
        .unwrap()
        .get_config()
        .general
        .mixed_port;
    let data_dir = match crate::util::paths::get_app_data_dir(app_handle) {
        Ok(dir) => dir,
        Err(e) => {
            error!(
                "Exit cleanup aborted before stopping Mihomo: cannot resolve proxy journal directory: {}",
                e
            );
            return;
        }
    };
    match crate::proxy::journal::release_owned_proxy_for_exit(&data_dir, mixed_port) {
        Ok(crate::proxy::journal::ReleaseOutcome::OwnershipLost) => {
            // 正常退出前已被用户/其他软件接管：本次不写注册表，同时把下次启动的
            // 自动接管意图关闭，避免 journal 清除后又把外部代理当作新 baseline。
            let mut cfg_mgr = state.config_manager.lock().unwrap();
            let mut cfg = cfg_mgr.get_config();
            if cfg.general.system_proxy {
                cfg.general.system_proxy = false;
                if let Err(e) = cfg_mgr.set_config(cfg) {
                    error!(
                        "Exit cleanup aborted before stopping Mihomo: failed to persist ownership loss: {}",
                        e
                    );
                    return;
                }
            }
        }
        Ok(_) => {}
        Err(e) => {
            error!(
                "Exit cleanup aborted before stopping Mihomo; proxy restore failed or ownership is ambiguous, journal kept: {}",
                e
            );
            return;
        }
    }

    // 复读验证已由 helper 完成。现在才允许停止本会话的 Mihomo。
    //    P1-7：只按 PID 精确清杀自己创建的进程。优先走 core_manager 锁拿
    //    实时 PID；锁被 async 任务占用时退回 supervisor 维护的 PID 缓存。
    //    两者都没有（本会话从未成功启动过核心）就什么都不杀——绝不按
    //    进程名 taskkill，避免误杀用户另行运行的 mihomo。
    let pid = {
        match state.core_manager.get() {
            Some(core) => core.child_pid(),
            None => None,
        }
    }
    .or_else(|| {
        let cached = state
            .core_pid_cache
            .load(std::sync::atomic::Ordering::SeqCst);
        if cached == 0 {
            None
        } else {
            Some(cached)
        }
    });
    if let Some(pid) = pid {
        info!("Killing mihomo (PID {}) on exit", pid);
        let stopped = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        if !stopped {
            error!(
                "Failed to stop Mihomo PID {} during exit; proxy was restored but journal is kept",
                pid
            );
            return;
        }
    } else {
        info!(
            "No mihomo child recorded this session; skipping kill \
             (will not touch unrelated mihomo processes)"
        );
    }

    // 核心已确认停止，最后清理退出凭据。
    crate::proxy::journal::clear_journal(&data_dir);
}

fn main() {
    run();
}
