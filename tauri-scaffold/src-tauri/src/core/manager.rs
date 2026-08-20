// src-tauri/src/core/manager.rs
//! CoreManager: mihomo 进程生命周期管理 + 外部控制器 API 客户端
//!
//! 单一数据源：`CoreManager` 与 `ConfigManager` 共享同一个
//! `Arc<parking_lot::RwLock<Config>>`，所有读取/修改走同一把锁。
//!
//! 运行架构：
//! - mihomo 以 `-d <data_dir> -f <data_dir>/runtime-config.yaml` 启动；
//!   runtime-config.yaml 由 `core::config::build_runtime_config`
//!   （AppConfig + 激活 Profile）生成，应用级字段不进入运行时配置。
//! - 启动后轮询 REST `/version` 就绪才置 Running，避免"启动假成功"；
//! - 子进程崩溃由 watcher 检测并置 Error + emit；
//! - 运行中重载走 REST `PUT /configs?force=true`（YAML payload），失败回退重启。
//!
//! 注意：所有 `std::sync::Mutex` guard 都不得跨 `.await` 持有。

use std::fmt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use parking_lot::RwLock;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use tokio::process::Child;
use tracing::{error, info, warn};

use crate::config::model::Config;
use crate::core::config::build_runtime_config;
use crate::util::error::{Error, Result};
use crate::util::paths::sanitize_profile_name;

/// mihomo 外部控制器就绪轮询的最大等待时间
const READY_TIMEOUT: Duration = Duration::from_secs(10);
/// 就绪轮询间隔
const READY_POLL_INTERVAL: Duration = Duration::from_millis(300);

/// Core 状态枚举
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum CoreStatus {
    /// 进程已停止
    #[default]
    Stopped,
    /// 正在启动中
    Starting,
    /// 运行中
    Running,
    /// 正在停止
    Stopping,
    /// 错误状态
    Error(String),
}

impl fmt::Display for CoreStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            CoreStatus::Stopped => "stopped",
            CoreStatus::Starting => "starting",
            CoreStatus::Running => "running",
            CoreStatus::Stopping => "stopping",
            CoreStatus::Error(e) => return write!(f, "error: {}", e),
        };
        f.write_str(s)
    }
}

/// CoreManager 管理 mihomo 进程 + 对外部控制器的 REST 调用
pub struct CoreManager {
    /// mihomo 子进程（Arc 供 watcher 任务共享；Deref 后原有 `.lock()` 用法不变）
    child: Arc<Mutex<Option<Child>>>,
    /// 共享配置（与 ConfigManager 同一 Arc，单一数据源）
    config: Arc<RwLock<Config>>,
    /// 当前状态
    status: Arc<Mutex<CoreStatus>>,
    /// 已获取的版本缓存（运行期间不再反复 `-v` 子进程）
    version_cache: Mutex<Option<String>>,
    /// mihomo 二进制路径
    mihomo_path: PathBuf,
    /// 初始化时 mihomo 缺失的可操作提示（Some 时 start() 直接以 Error 状态呈现，
    /// 而不是让整个应用启动失败——用户得能打开界面看到"核心缺失"的原因）
    init_error: Option<String>,
    /// 数据目录（含 runtime-config.yaml / profiles / 地理数据）
    data_dir: PathBuf,
    /// 外部控制器 HTTP 客户端
    api_client: reqwest::Client,
    /// AppHandle（用于状态变更事件推送）
    app_handle: AppHandle,
}

impl CoreManager {
    /// 创建新的 CoreManager 实例（与 ConfigManager 共享 config Arc）
    pub fn new(app_handle: AppHandle, config: Arc<RwLock<Config>>) -> Result<Self> {
        let data_dir = crate::util::paths::get_app_data_dir(&app_handle)?;

        // mihomo 缺失不阻断应用启动：get_mihomo_path 不再回退 %APPDATA%，
        // 缺失时记录可操作提示，start() 再以 Error 状态呈现给用户。
        // 诊断日志：明示便携/安装分支与最终解析路径，便于用户/排查定位
        // 是哪个目录在报错（旧版安装布局 sidecar/ 与便携布局 App/ 判别）。
        info!(
            "mihomo path resolution: {} -> {:?}",
            crate::util::paths::portable_mode_diagnostic(),
            crate::util::paths::get_mihomo_path(&app_handle).ok()
        );
        let (mihomo_path, init_error) = match crate::util::paths::get_mihomo_path(&app_handle) {
            Ok(path) => (path, None),
            Err(e) => {
                error!(
                    "mihomo not resolvable at startup: {} (mode: {})",
                    e,
                    crate::util::paths::portable_mode_diagnostic()
                );
                (PathBuf::new(), Some(e.to_string()))
            }
        };

        info!("mihomo path: {:?}", mihomo_path);

        Ok(CoreManager {
            child: Arc::new(Mutex::new(None)),
            config,
            status: Arc::new(Mutex::new(CoreStatus::Stopped)),
            version_cache: Mutex::new(None),
            mihomo_path,
            init_error,
            data_dir,
            api_client: reqwest::Client::builder()
                // 低危：REST 客户端统一超时，避免对控制器请求无限阻塞
                .timeout(Duration::from_secs(10))
                .build()?,
            app_handle,
        })
    }

    /// 获取当前状态
    pub fn status(&self) -> CoreStatus {
        self.status.lock().unwrap().clone()
    }

    /// 是否正在运行
    pub fn is_running(&self) -> bool {
        self.child.lock().unwrap().is_some()
    }

    /// 当前 mihomo 子进程 PID（同步读取，供退出清理使用）
    pub fn child_pid(&self) -> Option<u32> {
        self.child.lock().unwrap().as_ref().and_then(|c| c.id())
    }

    /// 获取配置快照（克隆，调用方自由持有）
    pub fn config(&self) -> Config {
        self.config.read().clone()
    }

    /// 运行时配置文件路径
    pub fn runtime_config_path(&self) -> PathBuf {
        self.data_dir.join("runtime-config.yaml")
    }

    // ---------- 配置 / 进程生命周期 ----------

    /// 生成并原子写入运行时配置（AppConfig + 激活 Profile → runtime-config.yaml）
    fn write_runtime_config(&self, config: &Config) -> Result<()> {
        let profile = self.read_active_profile(config);
        let runtime = build_runtime_config(config, profile.as_deref())?;
        let yaml = serde_yaml::to_string(&runtime)?;

        let path = self.runtime_config_path();
        // 原子写入：随机后缀临时文件 + 排他创建 + rename（见 util::atomic）
        crate::util::atomic::atomic_write(&path, yaml.as_bytes())?;
        info!("Runtime config written to {:?}", path);
        Ok(())
    }

    /// 读取激活 Profile 的原始内容（名称经过净化，防止路径穿越）
    fn read_active_profile(&self, config: &Config) -> Option<String> {
        let name = config.general.profile.trim();
        if name.is_empty() {
            return None;
        }
        let safe = sanitize_profile_name(name).ok()?;
        let path = self
            .data_dir
            .join("profiles")
            .join(format!("{}.yaml", safe));
        match std::fs::read_to_string(&path) {
            Ok(content) => Some(content),
            Err(_) => {
                warn!(
                    "Active profile '{}' not found at {:?}; using builtin preset",
                    safe,
                    path.display()
                );
                None
            }
        }
    }

    /// 启动 mihomo 进程：生成运行时配置 → `-d -f` 启动 → 轮询 REST 就绪
    pub async fn start(&self) -> Result<()> {
        if self.is_running() {
            self.stop().await?;
        }

        *self.status.lock().unwrap() = CoreStatus::Starting;

        let mihomo_path = self.mihomo_path.clone();
        let data_dir = self.data_dir.clone();

        // 初始化阶段已判定内核缺失：直接给出可操作提示（便携模式提示检查
        // App/clash-edge-core.exe；安装版提示 sidecar 打包），而不是裸路径。
        if let Some(hint) = self.init_error.clone() {
            *self.status.lock().unwrap() = CoreStatus::Error(hint.clone());
            return Err(Error::NotFound(hint));
        }

        if !mihomo_path.exists() {
            let hint = crate::util::paths::mihomo_missing_hint(&self.app_handle);
            *self.status.lock().unwrap() = CoreStatus::Error(hint.clone());
            return Err(Error::NotFound(hint));
        }

        // 用当前配置（含激活 Profile）生成运行时配置
        let config = self.config();
        self.write_runtime_config(&config)?;

        let runtime_config = self.runtime_config_path();
        let mut cmd = tokio::process::Command::new(&mihomo_path);
        cmd.arg("-d").arg(&data_dir).arg("-f").arg(&runtime_config);
        cmd.current_dir(&data_dir);

        // 捕获 stdout/stderr 到日志文件（崩溃时有据可查，不再是"日志无痕"）
        let logs_dir = data_dir.join("logs");
        let _ = std::fs::create_dir_all(&logs_dir);
        let stdout_path = logs_dir.join("mihomo-stdout.log");
        let stderr_path = logs_dir.join("mihomo-stderr.log");

        // 日志保留：上一会话日志超过阈值则轮转为 .old.log（保留排查线索），
        // 当前会话重新从头写，避免单个日志文件无限增长占满磁盘。
        const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024; // 5 MiB
        for path in [&stdout_path, &stderr_path] {
            if let Ok(meta) = std::fs::metadata(path) {
                if meta.len() > MAX_LOG_BYTES {
                    let _ = std::fs::rename(path, path.with_extension("old.log"));
                }
            }
        }

        let stdout_file = std::fs::File::create(&stdout_path).map_err(|e| {
            Error::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("create {}: {}", stdout_path.display(), e),
            ))
        })?;
        let stderr_file = std::fs::File::create(&stderr_path).map_err(|e| {
            Error::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("create {}: {}", stderr_path.display(), e),
            ))
        })?;
        cmd.stdout(std::process::Stdio::from(stdout_file))
            .stderr(std::process::Stdio::from(stderr_file));

        // mihomo 是控制台程序：不设 CREATE_NO_WINDOW 会在 Windows 上弹出
        // 黑色控制台窗口（用户报的"大黑框"）。
        #[cfg(target_os = "windows")]
        {
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }

        let child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                *self.status.lock().unwrap() = CoreStatus::Error(e.to_string());
                return Err(Error::from(e));
            }
        };
        *self.child.lock().unwrap() = Some(child);

        // 就绪探测：轮询 REST /version（mihomo 起不来的话这里会超时 → Error，不假成功）
        match self.wait_ready().await {
            Ok(()) => {
                // 端口冲突检测：REST 就绪只代表控制器可达，不代表 mixed-port / DNS
                // 已成功监听。旧版 Clash 仍占用 7890/9053 时，mihomo 会静默跳过监听，
                // 系统代理指向 127.0.0.1:7890 便随之失效——必须显式报错，而非假 Running
                // （坚持「界面状态 = Mihomo 实际状态」）。
                if let Err(bind_err) = self.detect_bind_conflict() {
                    let _ = self.stop().await; // 清掉僵尸进程（stop 会把状态置 Stopped）
                    *self.status.lock().unwrap() = CoreStatus::Error(bind_err.to_string());
                    let _ = self.app_handle.emit(
                        "core-status-changed",
                        serde_json::json!({ "status": format!("error: {}", bind_err) }),
                    );
                    return Err(bind_err);
                }
                *self.status.lock().unwrap() = CoreStatus::Running;
                // 缓存版本（REST 优先；失败回退 `-v`）
                if let Ok(v) = self.version().await {
                    *self.version_cache.lock().unwrap() = Some(v);
                }
                self.spawn_watcher();
                info!(
                    "mihomo started ({} -d {} -f {})",
                    mihomo_path.display(),
                    data_dir.display(),
                    runtime_config.display()
                );
                // 启动成功也推送状态事件（含 restart 场景），前端据此刷新核心状态与代理组。
                let _ = self.app_handle.emit(
                    "core-status-changed",
                    serde_json::json!({ "status": "running" }),
                );
                Ok(())
            }
            Err(e) => {
                *self.status.lock().unwrap() = CoreStatus::Error(e.to_string());
                let _ = self.stop().await; // 清掉起不来的僵尸进程
                Err(e)
            }
        }
    }

    /// 检测 mihomo 启动日志中的端口绑定失败（mixed-port / DNS 被其他进程占用）。
    /// mihomo 在 bind 失败时会 `level=error` 记录并静默跳过该监听，但进程不退出、
    /// 控制器照常就绪——不检查就会呈现「运行中但代理端口已死」的假象。
    /// 返回第一个 bind 错误的可读描述；无错误则 Ok（不误报其他非 bind 的 error）。
    fn detect_bind_conflict(&self) -> Result<()> {
        let log_path = self.data_dir.join("logs").join("mihomo-stdout.log");
        let Ok(content) = std::fs::read_to_string(&log_path) else {
            return Ok(());
        };
        match parse_bind_error(&content) {
            Some(msg) => Err(Error::Other(msg)),
            None => Ok(()),
        }
    }

    /// 轮询 REST `/version` 直到就绪或超时；期间若子进程提前退出则立即报错。
    async fn wait_ready(&self) -> Result<()> {
        let url = self.api_url(&["version"], None)?;
        let headers = self.api_headers();
        let deadline = tokio::time::Instant::now() + READY_TIMEOUT;

        loop {
            // 子进程是否已提前退出（配置错误 / 端口占用 / 崩溃）
            let exited = self
                .child
                .lock()
                .unwrap()
                .as_mut()
                .and_then(|c| c.try_wait().ok().flatten());
            if let Some(st) = exited {
                return Err(Error::Other(format!(
                    "mihomo exited during startup (code {:?})",
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
                Ok(resp) if resp.status().is_success() => {
                    return Ok(());
                }
                Ok(_) | Err(_) => {
                    if tokio::time::Instant::now() >= deadline {
                        return Err(Error::Other(
                            "mihomo did not become ready in time (external controller unreachable)"
                                .to_string(),
                        ));
                    }
                    tokio::time::sleep(READY_POLL_INTERVAL).await;
                }
            }
        }
    }

    /// 子进程 watcher：检测异常退出 → 状态置 Error + 推送事件。
    /// 轮询 `try_wait`（Child 由 CoreManager 持有，stop() 仍可 kill）。
    /// 退出时若系统代理仍指向本应用端口，立即关闭它——否则系统代理会继续
    /// 指向已死的 127.0.0.1:7890，所有网络请求 ERR_CONNECTION_REFUSED，
    /// 用户看起来像「断网」（BUG1 根因之三：内核崩溃后代理未自愈）。
    fn spawn_watcher(&self) {
        let status_arc = self.status.clone();
        let child_handle = self.child.clone();
        let app_handle = self.app_handle.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(500)).await;
                // 子进程是否已退出：try_wait 非空（进程自然退出/崩溃），
                // 或 child 已被 stop() 取走（None）。
                let gone = {
                    let mut guard = child_handle.lock().unwrap();
                    match guard.as_mut() {
                        Some(c) => c.try_wait().ok().flatten().is_some(),
                        None => true,
                    }
                };
                if gone {
                    // 置 child 为 None，is_running() 随之返回 false
                    child_handle.lock().unwrap().take();
                    let mut st = status_arc.lock().unwrap();
                    if *st != CoreStatus::Stopped && *st != CoreStatus::Stopping {
                        *st = CoreStatus::Error("mihomo exited unexpectedly".to_string());
                        error!("mihomo exited unexpectedly");
                        let _ = app_handle.emit(
                            "core-status-changed",
                            serde_json::json!({ "status": st.to_string() }),
                        );
                        // 系统代理自愈：内核已死，代理指向的端口也随之失效。
                        // 不关掉会让用户全网 ERR_CONNECTION_REFUSED。仅在配置
                        // 意图为「开」时关闭（用户主动关的无需再动），且只清
                        // ProxyEnable=0，不动 ProxyServer（退出还原由 main.rs 快照）。
                        let should_clear = {
                            let state = app_handle.state::<crate::AppState>();
                            let cfg_mgr = state.config_manager.lock().unwrap();
                            cfg_mgr.get_config().general.system_proxy
                        };
                        if should_clear {
                            warn!("Disabling system proxy: core crashed (was pointing at dead port)");
                            if let Err(e) = crate::proxy::system_proxy::set_system_proxy(
                                false, "", &[],
                            ) {
                                error!("Failed to disable system proxy after core crash: {}", e);
                            }
                        }
                    }
                    break;
                }
            }
        });
    }

    /// 停止 mihomo 进程
    pub async fn stop(&self) -> Result<()> {
        let child = self.child.lock().unwrap().take();
        if let Some(mut child) = child {
            info!("Stopping mihomo (PID {})", child.id().unwrap_or(0));
            *self.status.lock().unwrap() = CoreStatus::Stopping;
            // 先温和 kill，再兜底 taskkill（防句柄未回收导致杀不掉）
            let _ = child.kill().await;
            let _ = tokio::time::timeout(Duration::from_secs(3), child.wait()).await;
        }
        *self.status.lock().unwrap() = CoreStatus::Stopped;
        info!("mihomo stopped");
        Ok(())
    }

    /// 重启 mihomo
    pub async fn restart(&self) -> Result<()> {
        self.stop().await?;
        self.start().await
    }

    /// 重载配置：重新生成 runtime-config.yaml；运行中用 REST 热重载，
    /// REST 失败或未运行时回退整进程重启。
    pub async fn reload_config(&self) -> Result<()> {
        let config = self.config();
        self.write_runtime_config(&config)?;

        if self.is_running() {
            let yaml = std::fs::read_to_string(self.runtime_config_path())?;
            let payload = serde_json::json!({ "path": "", "payload": yaml });
            let url = self.api_url(&["configs"], Some(&[("force", "true")]))?;
            let resp = self
                .api_client
                .put(url)
                .headers(self.api_headers())
                .json(&payload)
                .send()
                .await;
            match resp {
                Ok(resp) if resp.status().is_success() => {
                    info!("Runtime config hot-reloaded via PUT /configs");
                    // 版本缓存失效，重取
                    if let Ok(v) = self.version().await {
                        *self.version_cache.lock().unwrap() = Some(v);
                    }
                    return Ok(());
                }
                Ok(resp) => {
                    warn!(
                        "REST reload returned {}, falling back to restart",
                        resp.status()
                    );
                }
                Err(e) => {
                    warn!("REST reload failed ({}), falling back to restart", e);
                }
            }
        }

        info!("Config rewritten, restarting core");
        self.restart().await
    }

    /// 用当前共享配置重新生成 runtime-config.yaml（不重启、不推事件）。
    /// 供编排层（core::runtime）在持久化后调用，保证下次启动/重载即用新值。
    pub fn regen_runtime_config(&self) -> Result<()> {
        let config = self.config();
        self.write_runtime_config(&config)
    }

    /// 获取版本信息：运行中优先 REST `/version`（缓存），否则 `mihomo -v` 兜底
    pub async fn version(&self) -> Result<String> {
        if self.is_running() {
            if let Some(v) = self.version_cache.lock().unwrap().clone() {
                return Ok(v);
            }
            let url = self.api_url(&["version"], None)?;
            if let Ok(resp) = self
                .api_client
                .get(url)
                .headers(self.api_headers())
                .send()
                .await
            {
                if resp.status().is_success() {
                    if let Ok(json) = resp.json::<serde_json::Value>().await {
                        if let Some(v) = json.get("version").and_then(|v| v.as_str()) {
                            let full = format!("mihomo {}", v);
                            *self.version_cache.lock().unwrap() = Some(full.clone());
                            return Ok(full);
                        }
                    }
                }
            }
        }

        let mihomo_path = self.mihomo_path.clone();
        let mut cmd = tokio::process::Command::new(&mihomo_path);
        cmd.arg("-v");
        #[cfg(target_os = "windows")]
        {
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let output = cmd.output().await?;
        let stderr_str = String::from_utf8_lossy(&output.stderr);
        let stdout_str = String::from_utf8_lossy(&output.stdout);
        let first = if !stderr_str.is_empty() {
            stderr_str.lines().next()
        } else {
            stdout_str.lines().next()
        };
        Ok(first.unwrap_or("unknown").to_string())
    }

    /// 当前状态 JSON（供前端）
    pub async fn get_status(&self) -> serde_json::Value {
        let status = self.status().to_string();
        let running = self.is_running();
        let version = if running {
            self.version().await.ok()
        } else {
            None
        };
        serde_json::json!({
            "running": running,
            "status": status,
            "version": version,
        })
    }

    // ---------- 外部控制器 API ----------

    /// 外部控制器基础地址（确保带 http://）
    fn api_base(&self) -> String {
        let addr = self.config.read().proxy.external_controller.clone();
        if addr.starts_with("http://") || addr.starts_with("https://") {
            addr
        } else {
            format!("http://{}", addr)
        }
    }

    fn api_headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        let secret = self.config.read().proxy.secret.clone();
        if !secret.is_empty() {
            if let Ok(v) = HeaderValue::from_str(&format!("Bearer {}", secret)) {
                headers.insert(AUTHORIZATION, v);
            }
        }
        headers
    }

    /// 构造控制器 URL；路径段逐段 percent-encode（组名/节点名可含空格、`/`、非 ASCII）。
    fn api_url(&self, path: &[&str], query: Option<&[(&str, &str)]>) -> Result<Url> {
        api_url(&self.api_base(), path, query)
    }

    /// 切换代理模式（PATCH /configs）——只作用于运行中的 mihomo；
    /// 持久化 / 回滚由编排层（apply_proxy_mode）负责。
    pub async fn set_proxy_mode(&self, mode: String) -> Result<()> {
        let url = self.api_url(&["configs"], None)?;
        let resp = self
            .api_client
            .patch(url)
            .headers(self.api_headers())
            .json(&serde_json::json!({ "mode": mode }))
            .send()
            .await?;

        if resp.status().is_success() {
            info!("Proxy mode set to {}", mode);
            Ok(())
        } else {
            warn!("Failed to set proxy mode: {}", resp.status());
            Err(Error::Other(format!(
                "Controller returned {}",
                resp.status()
            )))
        }
    }

    /// 运行中应用 TUN 开关（PATCH /configs {tun:{...}}）。
    /// TUN 变更在 mihomo 中通常需要完整 tun 段；失败时调用方回退整进程重启。
    pub async fn apply_tun(&self, enable: bool) -> Result<()> {
        let tun = self.config.read().tun.clone();
        let mut tun_value = serde_json::to_value(&tun).unwrap_or_default();
        if let Some(obj) = tun_value.as_object_mut() {
            obj.insert("enable".to_string(), serde_json::Value::Bool(enable));
        }
        let url = self.api_url(&["configs"], None)?;
        let resp = self
            .api_client
            .patch(url)
            .headers(self.api_headers())
            .json(&serde_json::json!({ "tun": tun_value }))
            .send()
            .await?;

        if resp.status().is_success() {
            info!(
                "TUN mode {} applied to running core",
                if enable { "enabled" } else { "disabled" }
            );
            Ok(())
        } else {
            warn!("Failed to apply TUN: {}", resp.status());
            Err(Error::Other(format!(
                "Controller returned {}",
                resp.status()
            )))
        }
    }

    /// 获取代理组列表（GET /proxies）。
    /// mihomo 返回的类型名是大写（Selector / URLTest / Fallback / LoadBalance / Relay），
    /// 旧实现只认小写导致永远匹配不到真实代理组。
    pub async fn get_proxy_groups(&self) -> Result<Vec<serde_json::Value>> {
        let url = self.api_url(&["proxies"], None)?;
        let resp = self
            .api_client
            .get(url)
            .headers(self.api_headers())
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(Error::Other(format!(
                "Controller returned {}",
                resp.status()
            )));
        }

        let json: serde_json::Value = resp.json().await?;
        let mut groups = Vec::new();
        if let Some(proxies) = json.get("proxies").and_then(|v| v.as_object()) {
            for (name, value) in proxies {
                let group_type = value
                    .get("type")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string();
                // 真实 mihomo 代理组类型（大小写不敏感，兼容小写旧写法）
                if ["Selector", "URLTest", "Fallback", "LoadBalance", "Relay"]
                    .iter()
                    .any(|t| t.eq_ignore_ascii_case(&group_type))
                {
                    let now = value
                        .get("now")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let all = value
                        .get("all")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    groups.push(serde_json::json!({
                        "name": name,
                        "type": group_type,
                        "now": now,
                        "all": all,
                    }));
                }
            }
        }
        Ok(groups)
    }

    /// 选择代理组中的某个代理（PUT /proxies/{group}，组名 URL 编码）
    pub async fn select_proxy_group(&self, group: String, proxy: String) -> Result<()> {
        let url = self.api_url(&["proxies", &group], None)?;
        let resp = self
            .api_client
            .put(url)
            .headers(self.api_headers())
            .json(&serde_json::json!({ "name": proxy }))
            .send()
            .await?;

        if resp.status().is_success() {
            info!("Selected {} -> {}", group, proxy);
            Ok(())
        } else {
            warn!("Failed to select proxy group: {}", resp.status());
            Err(Error::Other(format!(
                "Controller returned {}",
                resp.status()
            )))
        }
    }

    /// 测试代理组延迟（GET /proxies/{group}/delay）
    pub async fn test_proxy_latency(
        &self,
        group: String,
        url: Option<String>,
    ) -> Result<Vec<serde_json::Value>> {
        let test_url = url.unwrap_or_else(|| "http://www.gstatic.com/generate_204".to_string());
        // C2 SSRF 防护：该 URL 会作为参数传给 mihomo 由内核去拉取（非本地 client），
        // 同样必须通过禁段校验，防止被当作跳板探测内网。
        crate::util::fetch::validate_url(&test_url).await?;
        let api_url = self.api_url(
            &["proxies", &group, "delay"],
            Some(&[("url", test_url.as_str()), ("timeout", "5000")]),
        )?;

        let req = self
            .api_client
            .get(api_url)
            .headers(self.api_headers())
            .send()
            .await?;

        if req.status().is_success() {
            let body: serde_json::Value = req.json().await.unwrap_or_default();
            Ok(vec![serde_json::json!({
                "group": group,
                "delay": body.get("delay"),
            })])
        } else {
            Ok(vec![serde_json::json!({
                "group": group,
                "delay": null,
                "message": format!("HTTP {}", req.status()),
            })])
        }
    }

    /// 获取活动连接（GET /connections）
    /// 返回压缩后的连接列表 JSON（供前端连接面板显示）
    pub async fn get_connections(&self) -> Result<serde_json::Value> {
        let url = self.api_url(&["connections"], None)?;
        let resp = self
            .api_client
            .get(url)
            .headers(self.api_headers())
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(Error::Other(format!(
                "Controller returned {}",
                resp.status()
            )));
        }

        let json: serde_json::Value = resp.json().await?;
        let download_total = value_as_u64(json.get("downloadTotal"));
        let upload_total = value_as_u64(json.get("uploadTotal"));

        let mut connections = Vec::new();
        if let Some(arr) = json.get("connections").and_then(|v| v.as_array()) {
            for conn in arr {
                let metadata = conn.get("metadata");
                let host = metadata
                    .and_then(|m| m.get("host"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .or_else(|| {
                        metadata
                            .and_then(|m| m.get("remoteDestination"))
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                    })
                    .or_else(|| {
                        metadata
                            .and_then(|m| m.get("destinationIP"))
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                    })
                    .unwrap_or("")
                    .to_string();

                let network = metadata
                    .and_then(|m| m.get("network"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("tcp")
                    .to_string();

                let conn_type = metadata
                    .and_then(|m| m.get("type"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                let rule = conn
                    .get("rule")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let upload = value_as_u64(conn.get("upload"));
                let download = value_as_u64(conn.get("download"));
                let start = value_as_u64(conn.get("start"));
                let chains = conn
                    .get("chains")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();

                connections.push(serde_json::json!({
                    "id": conn.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    "host": host,
                    "network": network,
                    "type": conn_type,
                    "rule": rule,
                    "upload": upload,
                    "download": download,
                    "start": start,
                    "chains": chains,
                }));
            }
        }

        Ok(serde_json::json!({
            "download_total": download_total,
            "upload_total": upload_total,
            "connections": connections,
        }))
    }

    /// 关闭全部活动连接（DELETE /connections）
    pub async fn close_all_connections(&self) -> Result<()> {
        let url = self.api_url(&["connections"], None)?;
        let resp = self
            .api_client
            .delete(url)
            .headers(self.api_headers())
            .send()
            .await?;

        if resp.status().is_success() {
            info!("All connections closed");
            Ok(())
        } else {
            warn!("Failed to close connections: {}", resp.status());
            Err(Error::Other(format!(
                "Controller returned {}",
                resp.status()
            )))
        }
    }
}

/// 构造 mihomo 外部控制器 URL；路径段逐段 percent-encode
/// （组名/节点名可含空格、`/`、非 ASCII，直接拼接会生成非法 URL）。
fn api_url(base: &str, path: &[&str], query: Option<&[(&str, &str)]>) -> Result<Url> {
    let mut url = Url::parse(base)
        .map_err(|_| Error::InvalidArgument(format!("bad external-controller url: {}", base)))?;
    {
        let mut segs = url
            .path_segments_mut()
            .map_err(|_| Error::InvalidArgument("bad controller path".to_string()))?;
        for s in path {
            segs.push(s);
        }
    }
    if let Some(q) = query {
        let mut pairs = url.query_pairs_mut();
        for (k, v) in q {
            pairs.append_pair(k, v);
        }
    }
    Ok(url)
}

/// 从 JSON 值取 u64（兼容整数/浮点，缺失返回 0）
fn value_as_u64(v: Option<&serde_json::Value>) -> u64 {
    v.and_then(|v| v.as_u64())
        .or_else(|| v.and_then(|v| v.as_f64()).map(|f| f as u64))
        .unwrap_or(0)
}

/// 从 mihomo 启动日志文本中提取第一个端口绑定失败（bind）行，返回可读描述。
/// 只匹配 `level=error` 且包含 `bind` 的行（如 mixed-port / DNS 被其他进程占用），
/// 忽略规则拉取等其他 error，避免误报。
fn parse_bind_error(content: &str) -> Option<String> {
    content.lines().find_map(|line| {
        if line.contains("level=error") && line.contains("bind") {
            let detail = line
                .split("msg=")
                .nth(1)
                .unwrap_or_default()
                .trim_matches('"')
                .trim();
            Some(format!(
                "端口绑定失败：{}。请先关闭占用该端口的程序（如旧版 Clash.F.Win 仍在后台运行）后重试。",
                detail
            ))
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_core_status_display() {
        assert_eq!(CoreStatus::Stopped.to_string(), "stopped");
        assert_eq!(CoreStatus::Running.to_string(), "running");
        assert_eq!(CoreStatus::Starting.to_string(), "starting");
    }

    #[test]
    fn test_parse_bind_error_detects_port_conflict() {
        // 正常启动日志：无 level=error+bind → None
        let ok = "time=\"...\" level=info msg=\"RESTful API listening at: 127.0.0.1:50715\"\n";
        assert_eq!(parse_bind_error(ok), None);

        // 端口占用：应提取出端口号 + 可操作提示
        let conflict = "time=\"...\" level=error msg=\"Start Mixed(http+socks) server error: listen tcp 127.0.0.1:7890: bind: Only one usage of each socket address (protocol/network address/port) is normally permitted.\"\n";
        let msg = parse_bind_error(conflict).unwrap();
        assert!(msg.contains("7890"), "should name the port: {}", msg);
        assert!(
            msg.contains("关闭占用该端口的程序"),
            "actionable hint: {}",
            msg
        );

        // 规则拉取等其他 error（不含 bind）不误报
        let provider = "time=\"...\" level=error msg=\"[Provider] direct pull error: context deadline exceeded\"\n";
        assert_eq!(parse_bind_error(provider), None);
    }

    #[test]
    fn test_api_url_encodes_path_segments() {
        // 组名含空格、斜杠、非 ASCII → 逐段编码
        let url = api_url(
            "http://127.0.0.1:9090",
            &["proxies", "扶梯出行/香港"],
            Some(&[("timeout", "5000")]),
        )
        .unwrap();
        let s = url.to_string();
        assert!(
            s.contains("%2F"),
            "slash in group name must be encoded: {}",
            s
        );
        assert!(s.contains("timeout=5000"), "query preserved: {}", s);
        assert!(
            s.starts_with("http://127.0.0.1:9090/proxies/"),
            "base kept: {}",
            s
        );
    }
}
