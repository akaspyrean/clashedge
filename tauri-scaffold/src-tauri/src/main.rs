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
/// - `core_manager` 是 tokio Mutex（异步锁，可在 `.await` 期间持有）
/// - `config_manager` / `tray` 是 std Mutex（同步锁，禁止跨 `.await` 持有）
pub struct AppState {
    pub core_manager: tauri::async_runtime::Mutex<Option<CoreManager>>,
    pub config_manager: std::sync::Mutex<ConfigManager>,
    pub tray: std::sync::Mutex<Option<tauri::tray::TrayIcon>>,
    /// 启动前的系统代理快照，用于退出时还原（None = 启动时无外部代理，退出时直接清除）
    pub original_system_proxy:
        std::sync::Mutex<Option<crate::proxy::system_proxy::SystemProxyConfig>>,
    /// 日志流任务句柄（前端日志页启用/停止；std Mutex，abort 为同步调用）
    pub log_stream: std::sync::Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
}

impl AppState {
    fn new() -> Self {
        Self {
            core_manager: tauri::async_runtime::Mutex::new(None),
            config_manager: std::sync::Mutex::new(ConfigManager::new()),
            tray: std::sync::Mutex::new(None),
            original_system_proxy: std::sync::Mutex::new(None),
            log_stream: std::sync::Mutex::new(None),
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
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_opener::init())
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

                // 快照启动前的系统代理状态，用于退出时还原。
                // 若当前系统代理恰好指向我们自己的端口（127.0.0.1:<mixed-port>），
                // 视为上次会话退出异常残留（自愈），退出时直接清除而不是还原，
                // 避免把残留的死代理指向当作用户原有配置。
                let cfg = config_mgr.get_config();
                let ours = format!("127.0.0.1:{}", cfg.general.mixed_port);
                let current = crate::proxy::system_proxy::get_system_proxy().ok();
                let original = match &current {
                    Some(c) if c.enabled && c.address == ours => None,
                    other => other.clone(),
                };
                *state.original_system_proxy.lock().unwrap() = original.clone();
                match &original {
                    Some(orig) => info!(
                        "System proxy snapshot at startup: enabled={}, address={}",
                        orig.enabled, orig.address
                    ),
                    None => info!(
                        "No original system proxy to restore (or stale from previous session)"
                    ),
                }
            }

            // 初始化核心管理器（与 ConfigManager 共享同一个配置 Arc，单一数据源）
            {
                let state = app.state::<AppState>();
                let config_handle = { state.config_manager.lock().unwrap().config_handle() };
                let core_mgr = CoreManager::new(app.handle().clone(), config_handle)?;
                *state.core_manager.try_lock().unwrap() = Some(core_mgr);
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
            let window = WebviewWindowBuilder::new(
                app,
                "main",
                WebviewUrl::App("index.html".into()),
            )
            .title("ClashEdge")
            .inner_size(850.0, 603.0)
            .min_inner_size(850.0, 603.0)
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
                        let core_guard = state.core_manager.lock().await;
                        let ok = match core_guard.as_ref() {
                            Some(core) => core.start().await.is_ok(),
                            None => false,
                        };
                        drop(core_guard);
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
                        }
                    } else if sys_proxy_intent && !started {
                        // 内核没起来但配置里仍想开系统代理：保持关闭，否则会指向死端口。
                        // 配置里的意图保留为 true，内核后续手动启动时再开。
                        warn!(
                            "System proxy stays OFF: core failed to start (config intent=true)"
                        );
                    }
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
            crate::commands::config::reset_config,
            crate::commands::config::export_config,
            crate::commands::config::import_config,
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
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    // 挂接 RunEvent 生命周期：退出时清理（杀 mihomo、还原系统代理），
    // 否则 mihomo 变成孤儿进程继续代理、系统代理残留指向死端口，全网断开。
    app.run(|app_handle, event| {
        if let tauri::RunEvent::Exit = event {
            cleanup_on_exit(app_handle);
        }
    });
}

/// 退出清理：1) 杀掉 mihomo 子进程；2) 还原启动前快照的系统代理。
/// 放在 RunEvent::Exit 同步执行（taskkill + 注册表写入，不依赖 async runtime）。
fn cleanup_on_exit(app_handle: &tauri::AppHandle) {
    // 1. 停止 mihomo：孤儿 mihomo 会让用户以为退出了代理实际还在代理。
    //    优先用 PID 精确清杀；try_lock 可能因 async 任务持有锁而失败，
    //    兜底改用按进程名 taskkill，保证无论锁状态如何都能杀到 mihomo。
    let state = app_handle.state::<AppState>();
    let pid = {
        let guard = state.core_manager.try_lock();
        match guard {
            Ok(g) => g.as_ref().and_then(|c| c.child_pid()),
            Err(_) => None,
        }
    };
    if let Some(pid) = pid {
        info!("Killing mihomo (PID {}) on exit", pid);
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .status();
    } else {
        // 锁竞争失败（async 启动任务仍持有）或 core 未初始化：
        // 按进程名清杀兜底，防误杀上一轮孤儿 mihomo。
        // 两种内核名都清（参考包布局 mihomo-win64.exe / 原生便携 clash-edge-core.exe）。
        warn!("core_manager lock contended on exit; killing mihomo by name");
        for name in ["clash-edge-core.exe", "mihomo-win64.exe"] {
            let _ = std::process::Command::new("taskkill")
                .args(["/IM", name, "/F"])
                .status();
        }
    }

    // 2. 还原系统代理：若启动前用户已有代理则还原之，否则清除我们设置的代理。
    let original = state.original_system_proxy.lock().unwrap().clone();
    match original {
        Some(orig) if orig.enabled => {
            match crate::proxy::system_proxy::set_system_proxy(
                true,
                &orig.address,
                &orig.bypass_list,
            ) {
                Ok(()) => info!(
                    "System proxy restored to original ({}) on exit",
                    orig.address
                ),
                Err(e) => error!("Failed to restore system proxy on exit: {}", e),
            }
        }
        _ => match crate::proxy::system_proxy::set_system_proxy(false, "", &[]) {
            Ok(()) => info!("System proxy cleared on exit"),
            Err(e) => error!("Failed to clear system proxy on exit: {}", e),
        },
    }
}

fn main() {
    run();
}
