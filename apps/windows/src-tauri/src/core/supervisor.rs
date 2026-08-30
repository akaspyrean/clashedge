// src-tauri/src/core/supervisor.rs
//! CoreManager 的子进程监督（CoreSupervisor）实现：
//! watcher 任务、自动重启、崩溃熔断、PID 缓存、bind 错误日志解析。
//!
//! 拆分自 `core::manager`（原单文件实现）；`impl CoreManager` 可分布在
//! 多个文件中，公开方法路径（`crate::core::manager::CoreManager::*`）不变。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use parking_lot::RwLock;
use tauri::{AppHandle, Emitter, Manager};
use tokio::process::Child;
use tracing::{debug, error, info, warn};

use crate::config::model::Config;
use crate::core::controller::{api_url, authorization_headers};
use crate::core::health::parse_bind_error;
use crate::core::manager::{CoreManager, CoreStatus, READY_POLL_INTERVAL, READY_TIMEOUT};
use crate::util::error::{Error, Result};

/// 自动重启的初始退避间隔（按窗口内崩溃次数翻倍：2s, 4s, 8s）
const AUTO_RESTART_BACKOFF: Duration = Duration::from_secs(2);

// 崩溃熔断：若仅以"成功即清零"计数，「启动 2 秒 → 崩溃 → 重启 → 又 2 秒 →
// 又崩溃」会无限循环；因此采用时间窗计数 + 稳定运行才清零：
/// 崩溃统计窗口：窗口内异常退出达到上限即停止自动重启
const CRASH_WINDOW: Duration = Duration::from_secs(600); // 10 分钟
/// 窗口内允许的异常退出次数（第 3 次崩溃后放弃重启，进入 Error）
const MAX_CRASHES_IN_WINDOW: usize = 3;
/// 稳定运行判定：进程连续运行超过该时长后崩溃，才清空窗口计数
/// （避免把「长期正常服务后的一次偶发崩溃」也算进熔断窗口）
const STABLE_RUN_DURATION: Duration = Duration::from_secs(300); // 5 分钟

impl CoreManager {
    /// 把当前子进程 PID 同步到 AppState 缓存。
    /// 退出清理在 core_manager 锁被 async 任务占用时无法走 child_pid()，
    /// 改用该缓存做按 PID 精确清杀，避免按进程名 taskkill 误杀无关进程。
    pub(super) fn record_pid_cache(&self) {
        if let Some(pid) = self.child_pid() {
            self.set_pid_cache(pid);
        }
    }

    /// 把 PID 写入全局缓存（子进程确认退出后必须立即清空，防止 PID
    /// 被系统复用后误杀无关进程；stop 失败时保留以支持退出清理追踪）。
    pub(super) fn set_pid_cache(&self, pid: u32) {
        let state = self.app_handle.state::<crate::AppState>();
        state
            .core_pid_cache
            .store(pid, std::sync::atomic::Ordering::SeqCst);
    }

    /// 清空全局 PID 缓存（0 = 本会话未持有任何子进程）。
    pub(super) fn clear_pid_cache(&self) {
        let state = self.app_handle.state::<crate::AppState>();
        state
            .core_pid_cache
            .store(0, std::sync::atomic::Ordering::SeqCst);
    }

    /// 检测 mihomo 启动日志中的端口绑定失败（mixed-port / DNS 被其他进程占用）。
    /// mihomo 在 bind 失败时会 `level=error` 记录并静默跳过该监听，但进程不退出、
    /// 控制器照常就绪——不检查就会呈现「运行中但代理端口已死」的假象。
    /// 返回第一个 bind 错误的可读描述；无错误则 Ok（不误报其他非 bind 的 error）。
    pub(super) fn detect_bind_conflict(&self) -> Result<()> {
        let log_path = self.data_dir.join("logs").join("mihomo-stdout.log");
        let Ok(content) = std::fs::read_to_string(&log_path) else {
            return Ok(());
        };
        match parse_bind_error(&content) {
            Some(msg) => Err(Error::Other(msg)),
            None => Ok(()),
        }
    }

    /// 子进程监督循环（CoreSupervisor）：
    ///
    /// - 单 watcher 保证：每个 start() 只 spawn 一个携带 generation 的 watcher，
    ///   任何 start()/stop() 都会递增 generation 使旧 watcher 在下一次轮询时退出；
    /// - 崩溃处理链：状态置 Error → 关闭指向死端口的系统代理（防断网）→
    ///   circuit breaker 判定 → 退避后自动重启 → /version + mixed-port 健康检查
    ///   → 全部通过才恢复系统代理（若用户意图为开）；
    /// - 崩溃熔断：STABLE_RUN_DURATION 内的崩溃记入 CRASH_WINDOW 时间窗，
    ///   窗口内达到 MAX_CRASHES_IN_WINDOW 次即放弃重启；稳定运行超过阈值后的
    ///   崩溃清空窗口（长期正常服务后的偶发崩溃不受历史连坐）。
    pub(super) fn spawn_watcher(&self, generation: u64) {
        let status_arc = self.status.clone();
        let child_handle = self.child.clone();
        let app_handle = self.app_handle.clone();
        let user_stopped = self.user_stopped.clone();
        let gen_counter = self.generation.clone();
        let crash_times = self.crash_times.clone();
        let started_at = self.started_at.clone();
        let mihomo_path: PathBuf = self.mihomo_path.clone();
        let data_dir = self.data_dir.clone();
        let config = self.config.clone();
        let api_client = self.controller.api_client().clone();
        // 自愈重启成功后失效版本缓存（新进程版本可能已变化）
        let version_cache = self.version_cache.clone();

        // generation 是否仍然有效（不一致说明有新的 start/stop 接管）
        let gen_valid = |g: &Arc<std::sync::atomic::AtomicU64>, expected: u64| -> bool {
            g.load(std::sync::atomic::Ordering::SeqCst) == expected
        };

        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(500)).await;

                // generation 已变化 → 新流程接管，本 watcher 立即退出。
                // 这是消除「多个 watcher 同时盯一个 child、重复重启」的关键。
                if !gen_valid(&gen_counter, generation) {
                    debug!("supervisor watcher {} superseded; exiting", generation);
                    break;
                }

                // 子进程是否已退出：try_wait 非空（进程自然退出/崩溃），
                // 或 child 已被 stop() 取走（None）。
                let gone = {
                    let mut guard = child_handle.lock().unwrap();
                    match guard.as_mut() {
                        Some(c) => c.try_wait().ok().flatten().is_some(),
                        None => true,
                    }
                };
                if !gone {
                    continue;
                }

                // 置 child 为 None，is_running() 随之返回 false
                child_handle.lock().unwrap().take();
                // 子进程确认退出后必须立即清空 PID 缓存：否则崩溃/熔断/重启
                // 失败后旧 PID 可能被系统复用，应用退出清理会 taskkill 掉
                // 无关进程（PID 误杀风险）。
                {
                    let state = app_handle.state::<crate::AppState>();
                    state
                        .core_pid_cache
                        .store(0, std::sync::atomic::Ordering::SeqCst);
                }

                // 用户主动停止 / 正在停止：不重启，watcher 退出
                {
                    let st = status_arc.lock().unwrap();
                    if *st == CoreStatus::Stopped || *st == CoreStatus::Stopping {
                        break;
                    }
                }

                // 处理前再比对一次 generation（stop→start 快速交替的竞态窗口）
                if !gen_valid(&gen_counter, generation) {
                    break;
                }

                *status_arc.lock().unwrap() =
                    CoreStatus::Error("mihomo exited unexpectedly".to_string());
                error!("mihomo exited unexpectedly");
                let _ = app_handle.emit(
                    "core-status-changed",
                    serde_json::json!({
                        "status": "error: mihomo exited unexpectedly"
                    }),
                );

                // 系统代理自愈：内核已死时先按统一 ownership 语义精确恢复用户
                // 原状态。只有完整 managed 状态仍一致才写注册表；用户/其他软件
                // 已接管时绝不覆盖。
                let sys_proxy_intent = {
                    let state = app_handle.state::<crate::AppState>();
                    let cfg_mgr = state.config_manager.lock().unwrap();
                    cfg_mgr.get_config().general.system_proxy
                };
                let proxy_restore_target = if sys_proxy_intent {
                    let port = config.read().general.mixed_port;
                    match crate::proxy::journal::release_owned_proxy(&data_dir, port) {
                        Ok(crate::proxy::journal::ReleaseOutcome::Restored {
                            restored, ..
                        }) => Some(restored),
                        Ok(crate::proxy::journal::ReleaseOutcome::OwnershipLost)
                        | Ok(crate::proxy::journal::ReleaseOutcome::NoOwnership) => {
                            warn!(
                                "System proxy is no longer owned after core crash; preserving Windows state"
                            );
                            Self::give_up_system_proxy_after_restore_failure(
                                &app_handle,
                                &Error::Other("system proxy ownership changed".to_string()),
                            );
                            None
                        }
                        Err(e) => {
                            // ownership 不明确时不写注册表。若地址仍指向当前端口，
                            // 后续同端口重启会恢复服务；journal 保留供下次处理。
                            error!(
                                "Cannot safely release system proxy after core crash; journal kept: {}",
                                e
                            );
                            None
                        }
                    }
                } else {
                    None
                };

                if user_stopped.load(std::sync::atomic::Ordering::SeqCst) {
                    info!("User stopped core; not auto-restarting");
                    break;
                }

                // 崩溃熔断：
                // - 本次崩溃距启动不足 STABLE_RUN_DURATION → 记入窗口；
                //   超过 → 清空窗口（稳定运行后的偶发崩溃重新计数）
                // - 窗口内崩溃次数达到上限 → 放弃自动重启
                let now = std::time::Instant::now();
                let crashes_in_window = {
                    let mut times = crash_times.lock().unwrap();
                    let stable = started_at
                        .lock()
                        .unwrap()
                        .map(|s| now.duration_since(s) >= STABLE_RUN_DURATION)
                        .unwrap_or(false);
                    if stable {
                        times.clear();
                        info!(
                            "Core ran stable for >= {:?}; crash window reset",
                            STABLE_RUN_DURATION
                        );
                    }
                    times.retain(|t| now.duration_since(*t) <= CRASH_WINDOW);
                    times.push(now);
                    times.len()
                };
                if crashes_in_window >= MAX_CRASHES_IN_WINDOW {
                    error!(
                        "mihomo crashed {} times within {:?}; circuit breaker open, \
                         auto-restart abandoned",
                        crashes_in_window, CRASH_WINDOW
                    );
                    break;
                }
                let backoff = AUTO_RESTART_BACKOFF * (1u32 << (crashes_in_window - 1).min(4));
                warn!(
                    "Auto-restarting mihomo (crash {}/{} within {:?}) after {:?}",
                    crashes_in_window, MAX_CRASHES_IN_WINDOW, CRASH_WINDOW, backoff
                );
                tokio::time::sleep(backoff).await;

                // 退避期间可能发生了用户操作（stop/start），重启前再校验
                if !gen_valid(&gen_counter, generation) {
                    break;
                }

                // 尝试重启
                *status_arc.lock().unwrap() = CoreStatus::Starting;
                let _ = app_handle.emit(
                    "core-status-changed",
                    serde_json::json!({ "status": "starting" }),
                );
                if !mihomo_path.exists() {
                    error!("mihomo binary gone; cannot auto-restart");
                    *status_arc.lock().unwrap() =
                        CoreStatus::Error("mihomo binary missing; cannot restart".to_string());
                    break;
                }
                // 用当前配置生成运行时配置
                let cfg = config.read().clone();
                let runtime_config = data_dir.join("runtime-config.yaml");
                let profile = {
                    let name = cfg.general.profile.trim();
                    if name.is_empty() {
                        None
                    } else {
                        let safe = crate::util::paths::sanitize_profile_name(name).ok();
                        safe.and_then(|s| {
                            let p = data_dir.join("profiles").join(format!("{}.yaml", s));
                            std::fs::read_to_string(&p).ok()
                        })
                    }
                };
                match crate::core::config::build_runtime_config(&cfg, profile.as_deref()) {
                    Ok(runtime) => {
                        let yaml = match serde_yaml::to_string(&runtime) {
                            Ok(y) => y,
                            Err(e) => {
                                error!("Failed to serialize runtime config for restart: {}", e);
                                break;
                            }
                        };
                        if let Err(e) =
                            crate::util::atomic::atomic_write(&runtime_config, yaml.as_bytes())
                        {
                            error!("Failed to write runtime config for restart: {}", e);
                            break;
                        }
                    }
                    Err(e) => {
                        error!("Failed to build runtime config for restart: {}", e);
                        break;
                    }
                }
                let mut cmd = tokio::process::Command::new(&mihomo_path);
                cmd.arg("-d").arg(&data_dir).arg("-f").arg(&runtime_config);
                cmd.current_dir(&data_dir);
                let logs_dir = data_dir.join("logs");
                let _ = std::fs::create_dir_all(&logs_dir);
                let stdout_file = match std::fs::File::create(logs_dir.join("mihomo-stdout.log")) {
                    Ok(f) => f,
                    Err(e) => {
                        error!("Failed to create stdout log: {}", e);
                        break;
                    }
                };
                let stderr_file = match std::fs::File::create(logs_dir.join("mihomo-stderr.log")) {
                    Ok(f) => f,
                    Err(e) => {
                        error!("Failed to create stderr log: {}", e);
                        break;
                    }
                };
                cmd.stdout(std::process::Stdio::from(stdout_file))
                    .stderr(std::process::Stdio::from(stderr_file));
                #[cfg(target_os = "windows")]
                {
                    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
                }
                match cmd.spawn() {
                    Ok(child) => {
                        // 同步 PID 缓存
                        if let Some(pid) = child.id() {
                            app_handle
                                .state::<crate::AppState>()
                                .core_pid_cache
                                .store(pid, std::sync::atomic::Ordering::SeqCst);
                        }
                        *child_handle.lock().unwrap() = Some(child);
                        // 就绪探测：/version 可达 + 代理端口健康后才算恢复
                        let core_manager_for_check = AutoRestartChecker {
                            child: child_handle.clone(),
                            api_client: api_client.clone(),
                            config: config.clone(),
                        };
                        match core_manager_for_check.wait_ready_and_check_port().await {
                            Ok(()) => {
                                *status_arc.lock().unwrap() = CoreStatus::Running;
                                // 新进程的稳定运行基准从现在起算
                                *started_at.lock().unwrap() = Some(std::time::Instant::now());
                                let _ = app_handle.emit(
                                    "core-status-changed",
                                    serde_json::json!({ "status": "running" }),
                                );
                                // 恢复 Windows 系统代理。崩溃时已临时关闭；
                                // 自愈成功后必须按用户意图与真实端口恢复，否则出现
                                // 「UI=开、配置=开、注册表=关」的三态分裂。
                                if let Some(expected) = proxy_restore_target.as_ref() {
                                    let port = config.read().general.mixed_port;
                                    match crate::proxy::journal::acquire_system_proxy_if_unchanged(
                                        &data_dir,
                                        port,
                                        Some(expected),
                                    ) {
                                        Ok(()) => info!(
                                            "System proxy safely reacquired after auto-restart (127.0.0.1:{})",
                                            port
                                        ),
                                        Err(e) => {
                                            error!(
                                                "Failed to restore system proxy after \
                                                 auto-restart: {}",
                                                e
                                            );
                                            // 恢复失败不允许 UI 继续把系统代理当作 ON。
                                            // 尽力退回 journal 记录的用户原始代理状态；无论
                                            // 成败，配置意图都改回 false（= Windows 实际），
                                            // 并推送事件刷新前端，杜绝三态分裂。
                                            Self::give_up_system_proxy_after_restore_failure(
                                                &app_handle,
                                                &e,
                                            );
                                        }
                                    }
                                } else if sys_proxy_intent {
                                    // release 失败时可能是注册表读取/验证的瞬时错误。
                                    // 核心恢复后复读：仍是完整 managed 状态则保持 ON；
                                    // 否则仅把配置意图落回 false，不覆盖未知 Windows 状态。
                                    let port = config.read().general.mixed_port;
                                    let managed =
                                        crate::proxy::system_proxy::managed_proxy_config(port);
                                    if crate::proxy::system_proxy::get_system_proxy()
                                        .map(|current| current != managed)
                                        .unwrap_or(true)
                                    {
                                        Self::give_up_system_proxy_after_restore_failure(
                                            &app_handle,
                                            &Error::Other(
                                                "system proxy ownership could not be confirmed after core restart"
                                                    .to_string(),
                                            ),
                                        );
                                    }
                                }
                                info!("mihomo auto-restarted successfully");
                                // 版本缓存失效：新进程的 /version 可能与旧缓存不同，
                                // 与热重载路径对齐，下次 version() 重新获取。
                                *version_cache.lock().unwrap() = None;
                                // 继续监视新进程（同一 generation、同一 watcher）
                                continue;
                            }
                            Err(e) => {
                                error!("mihomo auto-restart failed readiness check: {}", e);
                                *status_arc.lock().unwrap() = CoreStatus::Error(e.to_string());
                                let _ = app_handle.emit(
                                    "core-status-changed",
                                    serde_json::json!({ "status": format!("error: {}", e) }),
                                );
                                // 清掉起不来的僵尸进程（先取 child 再 await）
                                let zombie = child_handle.lock().unwrap().take();
                                if let Some(mut c) = zombie {
                                    let _ = c.kill().await;
                                    // 僵尸进程被取走并清理后，PID 缓存必须清空，
                                    // 否则旧 PID 残留、应用退出时可能误杀被复用的进程。
                                    let state = app_handle.state::<crate::AppState>();
                                    state
                                        .core_pid_cache
                                        .store(0, std::sync::atomic::Ordering::SeqCst);
                                }
                                // 继续循环：下一次 gone 检测会再记一次崩溃并重试，
                                // 直至熔断窗口打开
                                continue;
                            }
                        }
                    }
                    Err(e) => {
                        error!("Failed to spawn mihomo on auto-restart: {}", e);
                        *status_arc.lock().unwrap() = CoreStatus::Error(e.to_string());
                        break;
                    }
                }
            }
        });
    }

    /// 自愈重启后系统代理恢复失败的收尾。
    ///
    /// 前提：崩溃恢复确认 ownership 已释放或重新接管失败，但配置意图仍为 true。
    /// 此时绝不能让 UI 继续把系统代理当作 ON——
    /// ownership helper 已负责安全恢复/保留 journal；这里仅把配置意图改回 false
    /// 并推送事件，绝不再直接写注册表形成第四套恢复逻辑。
    fn give_up_system_proxy_after_restore_failure(app_handle: &AppHandle, err: &Error) {
        {
            let state = app_handle.state::<crate::AppState>();
            let mut cfg_mgr = state.config_manager.lock().unwrap();
            let mut cfg = cfg_mgr.get_config();
            cfg.general.system_proxy = false;
            if let Err(e) = cfg_mgr.set_config(cfg) {
                error!(
                    "Failed to persist system_proxy=false after restore failure: {}",
                    e
                );
            }
        }
        let _ = app_handle.emit(
            "system-proxy-changed",
            serde_json::json!({ "enable": false, "error": err.to_string() }),
        );
    }
}

/// 自动重启后的就绪探测辅助（复用 CoreManager 的 /version + 端口健康检查）。
/// 不直接用 CoreManager 方法（避免借走 self 跨 await 与 watcher 任务所有权冲突），
/// 只取必要的 Arc 字段。
struct AutoRestartChecker {
    child: Arc<Mutex<Option<Child>>>,
    api_client: reqwest::Client,
    config: Arc<RwLock<Config>>,
}

impl AutoRestartChecker {
    /// 轮询 /version 就绪 + 代理端口健康检查，全部通过才返回 Ok。
    async fn wait_ready_and_check_port(&self) -> Result<()> {
        let addr = self.config.read().proxy.external_controller.clone();
        let base = if addr.starts_with("http://") || addr.starts_with("https://") {
            addr
        } else {
            format!("http://{}", addr)
        };
        let url = api_url(&base, &["version"], None)?;
        let secret = self.config.read().proxy.secret.clone();
        // 与 CoreManager::api_headers 同源：非法密钥字符显式报错，不静默省略
        let headers = authorization_headers(&secret)?;
        let deadline = tokio::time::Instant::now() + READY_TIMEOUT;
        loop {
            let exited = self
                .child
                .lock()
                .unwrap()
                .as_mut()
                .and_then(|c| c.try_wait().ok().flatten());
            if let Some(st) = exited {
                return Err(Error::Other(format!(
                    "mihomo exited during auto-restart startup (code {:?})",
                    st.code()
                )));
            }
            match self
                .api_client
                .get(url.clone())
                .headers(headers.clone())
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => break,
                Ok(_) | Err(_) => {
                    if tokio::time::Instant::now() >= deadline {
                        return Err(Error::Other(
                            "mihomo did not become ready after auto-restart".to_string(),
                        ));
                    }
                    tokio::time::sleep(READY_POLL_INTERVAL).await;
                }
            }
        }
        // 端口健康检查：代理端口可达才恢复系统代理
        let mixed_port = self.config.read().general.mixed_port;
        let proxy_addr = format!("127.0.0.1:{}", mixed_port);
        match tokio::net::TcpStream::connect(&proxy_addr).await {
            Ok(_) => Ok(()),
            Err(e) => Err(Error::Other(format!(
                "mihomo ready but mixed-port {} not listening: {}",
                mixed_port, e
            ))),
        }
    }
}
