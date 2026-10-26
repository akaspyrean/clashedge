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
//! 本文件只保留：类型定义（CoreStatus / CoreManager 结构体）、构造、
//! 状态/元数据访问器与 REST 转发外观。按单一职责拆分的实现模块：
//! - `core::lifecycle`  —— start/stop/restart/reload 进程生命周期；
//! - `core::supervisor` —— watcher 监督任务、自动重启、崩溃熔断、PID 缓存。
//!
//! 注意：所有 `std::sync::Mutex` guard 都不得跨 `.await` 持有。

use std::fmt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tokio::process::Child;
use tracing::{error, info};

use crate::config::model::Config;
use crate::core::controller::ControllerClient;
use crate::util::error::Result;

/// mihomo 外部控制器就绪轮询的最大等待时间（生命周期 start 与 supervisor
/// 自动重启的就绪探测共用）
pub(super) const READY_TIMEOUT: Duration = Duration::from_secs(10);
/// 就绪轮询间隔（同上共用）
pub(super) const READY_POLL_INTERVAL: Duration = Duration::from_millis(300);

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
    pub(super) child: Arc<Mutex<Option<Child>>>,
    /// 共享配置（与 ConfigManager 同一 Arc，单一数据源）
    pub(super) config: Arc<RwLock<Config>>,
    /// 当前状态
    pub(super) status: Arc<Mutex<CoreStatus>>,
    /// 已获取的版本缓存（运行期间不再反复 `-v` 子进程）。
    /// Arc 供 watcher 任务共享：自愈重启成功后必须失效（新进程版本可能不同）。
    pub(super) version_cache: Arc<Mutex<Option<String>>>,
    /// mihomo 二进制路径
    pub(super) mihomo_path: PathBuf,
    /// 初始化时 mihomo 缺失的可操作提示（Some 时 start() 直接以 Error 状态呈现，
    /// 而不是让整个应用启动失败——用户得能打开界面看到"核心缺失"的原因）
    pub(super) init_error: Option<String>,
    /// 数据目录（含 runtime-config.yaml / profiles / 地理数据）
    pub(super) data_dir: PathBuf,
    /// 外部控制器 REST 客户端（Config + HTTP client 都封装在此，
    /// 与进程生命周期字段解耦，见 core/controller.rs）
    pub(super) controller: ControllerClient,
    /// 会话级控制器地址覆盖（与 ControllerClient 共享；不进入共享配置）
    pub(super) controller_override: Arc<RwLock<Option<String>>>,
    /// AppHandle（用于状态变更事件推送）
    pub(super) app_handle: AppHandle,
    /// 用户主动停止标志（stop() 设为 true，start() 清除）。
    /// watcher 检测到崩溃时检查此标志：用户主动停止时不自动重启。
    pub(super) user_stopped: Arc<std::sync::atomic::AtomicBool>,
    /// supervisor generation：每次 start()/stop() 递增。
    /// watcher 捕获自己启动时的 generation，处理事件前先比对——不一致说明
    /// 已有新的 start/stop 流程接管，本 watcher 立即退出。保证任意时刻
    /// 至多一个有效 watcher（否则 stop→start 快速交替时旧 watcher 会盯上
    /// 新 child，崩溃时多个 watcher 同时重启）。
    pub(super) generation: Arc<std::sync::atomic::AtomicU64>,
    /// 崩溃熔断：窗口内的崩溃时间戳
    pub(super) crash_times: Arc<Mutex<Vec<std::time::Instant>>>,
    /// 当前子进程的启动时刻（稳定运行判定用）
    pub(super) started_at: Arc<Mutex<Option<std::time::Instant>>>,
    /// 最近一次成功应用到运行中核心的 runtime-config 内容哈希。
    /// reload 时哈希未变则跳过热重载（审计 B8）。
    pub(super) applied_hash: Arc<Mutex<Option<String>>>,
    /// 生命周期互斥锁：start/stop/restart/reload_config 串行执行，
    /// 只读 REST 操作（get_connections/get_proxy_groups/version 等）不需要此锁。
    pub(super) lifecycle: Arc<tokio::sync::Mutex<()>>,
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

        let controller = ControllerClient::new(config.clone())?;
        Ok(CoreManager {
            child: Arc::new(Mutex::new(None)),
            // 共享配置 Arc 需要 clone 一份给 CoreManager 字段、一份给 ControllerClient
            config: config.clone(),
            status: Arc::new(Mutex::new(CoreStatus::Stopped)),
            version_cache: Arc::new(Mutex::new(None)),
            mihomo_path,
            init_error,
            data_dir,
            // REST 客户端（config Arc + HTTP client）收敛到 ControllerClient
            controller_override: controller.override_handle(),
            controller,
            app_handle,
            user_stopped: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            generation: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            crash_times: Arc::new(Mutex::new(Vec::new())),
            started_at: Arc::new(Mutex::new(None)),
            applied_hash: Arc::new(Mutex::new(None)),
            lifecycle: Arc::new(tokio::sync::Mutex::new(())),
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

    /// 当前 mihomo 二进制路径（供退出清理做 PID → 映像路径所有权校验）
    pub fn mihomo_binary_path(&self) -> PathBuf {
        self.mihomo_path.clone()
    }

    /// 获取配置快照（克隆，调用方自由持有）
    /// 控制器会话覆盖已应用到 `proxy.external_controller`（用于生成 runtime-config / 预检）。
    pub fn config(&self) -> Config {
        let mut cfg = self.config.read().clone();
        if let Some(addr) = self.controller_override.read().clone() {
            cfg.proxy.external_controller = addr;
        }
        cfg
    }

    /// 当前生效的外部控制器地址（日志流等需要直连控制器的调用方使用）。
    pub fn controller_addr(&self) -> String {
        self.controller.effective_addr()
    }

    /// 运行时配置文件路径
    pub fn runtime_config_path(&self) -> PathBuf {
        self.data_dir.join("runtime-config.yaml")
    }

    /// 获取版本信息：运行中优先 REST `/version`（缓存），否则 `mihomo -v` 兜底
    pub async fn version(&self) -> Result<String> {
        if self.is_running() {
            if let Some(v) = self.version_cache.lock().unwrap().clone() {
                return Ok(v);
            }
            let url = self.controller.api_url(&["version"], None)?;
            if let Ok(resp) = self
                .controller
                .api_client()
                .get(url)
                .headers(self.controller.api_headers()?)
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

    // ---------- 外部控制器 API（逻辑在 core/controller.rs，这里转发） ----------

    /// 切换代理模式（PATCH /configs）——只作用于运行中的 mihomo；
    /// 持久化 / 回滚由编排层（AppController::apply_proxy_mode）负责。
    pub async fn set_proxy_mode(&self, mode: String) -> Result<()> {
        self.controller.set_proxy_mode(mode).await
    }

    /// 运行中应用 TUN 开关（PATCH /configs {tun:{...}}）。
    /// TUN 变更在 mihomo 中通常需要完整 tun 段；失败时调用方回退整进程重启。
    pub async fn apply_tun(&self, enable: bool) -> Result<()> {
        self.controller.apply_tun(enable).await
    }

    /// 读取运行中核心的实际 TUN 状态（GET /configs → tun.enable）。
    /// 供编排层 apply_tun 确认「PATCH/restart 后 Mihomo 是否真正接受了目标状态」。
    pub async fn get_tun_enable(&self) -> Result<bool> {
        self.controller.get_tun_enable().await
    }

    /// 获取代理组列表（GET /proxies）。
    pub async fn get_proxy_groups(&self) -> Result<Vec<serde_json::Value>> {
        let mut groups = self.controller.get_proxy_groups().await?;
        // 节点名不含国家信息时（例如 JMS-1416430@c37s1...），用 server 的 IP
        // 查本地 Country.mmdb 兜底；失败/数据库缺失时保持名称推断结果。
        self.enrich_regions_from_geoip(&mut groups).await;
        Ok(groups)
    }

    /// 为「名称推断不出地区」的节点补充 GeoIP 地区。
    ///
    /// 只解析缺失项并按主机名缓存结果，正常轮询不会重复 DNS / 查库。
    async fn enrich_regions_from_geoip(&self, groups: &mut [serde_json::Value]) {
        let db = self.data_dir.join("Country.mmdb");
        if !db.is_file() {
            return;
        }
        let servers = crate::util::node_servers::load(&self.data_dir.join("runtime-config.yaml"));
        if servers.is_empty() {
            return;
        }

        // TUN + fake-ip 时系统解析会返回假地址，需要切到 DoH 取真实 IP。
        let fake_ip_range = {
            let cfg = self.config.read();
            crate::util::fetch::fake_ip_range_from_config(&cfg)
        };

        // 收集：名称没有地区、但 runtime-config 里有 server 的节点（去重）。
        let mut pending: Vec<(String, String)> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for group in groups.iter() {
            let Some(all) = group.get("all").and_then(|v| v.as_array()) else {
                continue;
            };
            let regions = group.get("regions").and_then(|v| v.as_object());
            for node in all {
                let Some(name) = node.as_str() else {
                    continue;
                };
                if regions.map(|m| m.contains_key(name)).unwrap_or(false) {
                    continue;
                }
                if !seen.insert(name.to_string()) {
                    continue;
                }
                if let Some(server) = servers.get(name) {
                    pending.push((name.to_string(), server.clone()));
                }
            }
        }
        if pending.is_empty() {
            return;
        }

        let semaphore = Arc::new(tokio::sync::Semaphore::new(
            crate::util::geoip::MAX_CONCURRENT_LOOKUPS,
        ));
        let mut tasks = tokio::task::JoinSet::new();
        for (name, server) in pending {
            let db = db.clone();
            let semaphore = semaphore.clone();
            tasks.spawn(async move {
                let _permit = semaphore.acquire_owned().await.ok()?;
                let code = crate::util::geoip::country_code_for_server(
                    &db,
                    &server,
                    fake_ip_range.as_ref(),
                )
                .await?;
                let display = crate::util::region::display_for_code(&code)
                    .map(str::to_string)
                    .unwrap_or_else(|| code.to_ascii_uppercase());
                Some((name, display))
            });
        }

        let mut resolved = std::collections::HashMap::new();
        // 总预算：节点很多且 DNS 很慢时，本次先返回已解析的部分，剩余节点
        // 下次打开页面继续补（成功结果已按主机缓存，不会重复打 DNS）。
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            match tokio::time::timeout_at(deadline, tasks.join_next()).await {
                Ok(Some(Ok(Some((name, display))))) => {
                    resolved.insert(name, display);
                }
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(_) => {
                    tasks.abort_all();
                    break;
                }
            }
        }
        if resolved.is_empty() {
            return;
        }
        merge_resolved_regions(groups, &resolved);
    }
}

/// 把 GeoIP 解析结果合并进每个组的 `regions`（保留已有的名称推断结果）。
fn merge_resolved_regions(
    groups: &mut [serde_json::Value],
    resolved: &std::collections::HashMap<String, String>,
) {
    if resolved.is_empty() {
        return;
    }
    for group in groups.iter_mut() {
        let Some(all) = group.get("all").and_then(|v| v.as_array()) else {
            continue;
        };
        let mut regions = group
            .get("regions")
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();
        let mut changed = false;
        for node in all {
            let Some(name) = node.as_str() else {
                continue;
            };
            if let Some(display) = resolved.get(name) {
                regions.insert(name.to_string(), serde_json::Value::String(display.clone()));
                changed = true;
            }
        }
        if changed {
            if let Some(obj) = group.as_object_mut() {
                obj.insert("regions".to_string(), serde_json::Value::Object(regions));
            }
        }
    }
}

impl CoreManager {
    /// 选择代理组中的某个代理（PUT /proxies/{group}，组名 URL 编码）
    pub async fn select_proxy_group(&self, group: String, proxy: String) -> Result<()> {
        self.controller.select_proxy_group(group, proxy).await
    }

    /// 测试代理组延迟（GET /proxies/{group}/delay）
    pub async fn test_proxy_latency(
        &self,
        group: String,
        url: Option<String>,
    ) -> Result<Vec<serde_json::Value>> {
        self.controller.test_proxy_latency(group, url).await
    }

    /// 获取活动连接（GET /connections）
    pub async fn get_connections(&self) -> Result<serde_json::Value> {
        self.controller.get_connections().await
    }

    /// 获取规则列表（GET /rules，已裁剪为 type / payload / proxy）
    pub async fn get_rules(&self) -> Result<Vec<serde_json::Value>> {
        self.controller.get_rules().await
    }

    /// 关闭全部活动连接（DELETE /connections）
    pub async fn close_all_connections(&self) -> Result<()> {
        self.controller.close_all_connections().await
    }
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
    fn merges_geoip_regions_and_keeps_name_based_ones() {
        let mut groups = vec![
            serde_json::json!({
                "name": "人工优选",
                "all": ["JMS-1", "HK-01", "DIRECT"],
                "regions": { "HK-01": "香港" }
            }),
            serde_json::json!({ "name": "自动优选", "all": ["JMS-1"] }),
        ];
        let resolved = std::collections::HashMap::from([("JMS-1".to_string(), "美国".to_string())]);

        merge_resolved_regions(&mut groups, &resolved);

        assert_eq!(groups[0]["regions"]["HK-01"], "香港");
        assert_eq!(groups[0]["regions"]["JMS-1"], "美国");
        assert!(groups[0]["regions"].get("DIRECT").is_none());
        assert_eq!(groups[1]["regions"]["JMS-1"], "美国");
    }
}
