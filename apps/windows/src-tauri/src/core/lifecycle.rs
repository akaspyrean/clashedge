// src-tauri/src/core/lifecycle.rs
//! CoreManager 的进程生命周期实现：start / stop / restart / reload。
//!
//! 拆分自 `core::manager`（原单文件实现）；`impl CoreManager` 可分布在
//! 多个文件中，公开方法路径（`crate::core::manager::CoreManager::*`）不变。
//!
//! tokio Mutex 不可重入：公开入口持 lifecycle 锁后调用内部 stop()/start()
//! 会再次 lock 同一把锁 → 永久死锁。因此公开入口只加锁、内部 *_locked
//! 不再加锁，串行语义不变。
//!
//! 进程准备/启动细节（runtime-config、日志、孤儿回收、端口预检）见 `core::spawn`，
//! 首次启动与崩溃自动重启共用同一实现。

use std::time::Duration;

use tauri::Emitter;
use tokio::process::Child;
use tracing::{debug, error, info, warn};

use crate::config::model::Config;
use crate::core::health::{normalize_dns_listen, probe_str_addr, probe_tcp};
use crate::core::manager::{CoreManager, CoreStatus, READY_POLL_INTERVAL, READY_TIMEOUT};
use crate::core::spawn;
use crate::util::error::{Error, Result};

/// mihomo 端口健康探测超时（probe_tcp 的默认时长，混合端口 + DNS 探测共用）
const MIXED_PORT_PROBE_TIMEOUT: Duration = Duration::from_secs(3);

impl CoreManager {
    /// 生成并原子写入运行时配置（AppConfig + 激活 Profile → runtime-config.yaml），
    /// 返回内容哈希（用于判断热重载是否有实质变化）。
    fn write_runtime_config(&self, config: &Config) -> Result<String> {
        spawn::write_runtime_config(&self.data_dir, config).map(|(_, hash)| hash)
    }

    /// 进入可解释的 Error 状态并推送 `core-status-changed`。
    /// 所有启动/停止失败路径都必须以本方法收尾，保证 UI 绝不呈现
    /// 假运行 / 假停止 / 永久 Starting。
    fn transition_to_error_and_emit(&self, err: Error) -> Error {
        let text = err.to_string();
        error!("Core entered error state: {}", text);
        *self.status.lock().unwrap() = CoreStatus::Error(text.clone());
        let _ = self.app_handle.emit(
            "core-status-changed",
            serde_json::json!({ "status": format!("error: {}", text) }),
        );
        err
    }

    /// 轮询 try_wait 直到子进程确认退出或超时。try_wait 可重复调用
    /// （wait() 在超时取消后不能可靠二次调用），用于 stop_locked /
    /// 自动重启失败清理后确认进程确实消失（停止假成功防护）。
    async fn confirm_child_exit(&self, child: &mut Child, timeout: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            match child.try_wait() {
                Ok(Some(_)) => return true,
                Ok(None) => {}
                Err(_) => return false,
            }
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// 启动 mihomo 进程：回收孤儿 → 端口预检 → 生成运行时配置 → `-d -f` 启动 → 轮询 REST 就绪
    pub async fn start(&self) -> Result<()> {
        let _lifecycle_guard = self.lifecycle.lock().await;
        self.start_locked().await
    }

    /// start 实现体（调用方必须已持有 lifecycle 锁）。
    async fn start_locked(&self) -> Result<()> {
        // 幂等启动：若已在运行则直接返回，且不得在此分支调用 stop_locked()——
        // 那会让 generation 被二次递增，而下方 spawn_watcher(generation) 用的是
        // 递增前的旧值，导致 watcher 首次轮询即判定被取代而退出，核心失去
        // supervisor。start 的语义是"确保运行中"，重复调用不应产生副作用。
        if self.is_running() {
            return Ok(());
        }

        // 递增 generation 使所有旧 watcher 失效（它们会在下一次
        // 轮询比对时退出），随后本流程成功后只 spawn 一个新 watcher。
        let generation = self
            .generation
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            + 1;

        // 用户主动启动 → 清除停止标志
        self.user_stopped
            .store(false, std::sync::atomic::Ordering::SeqCst);

        *self.status.lock().unwrap() = CoreStatus::Starting;

        let mihomo_path = self.mihomo_path.clone();
        let data_dir = self.data_dir.clone();

        // 初始化阶段已判定内核缺失：直接给出可操作提示（便携模式提示检查
        // App/clash-edge-core.exe；安装版提示 sidecar 打包），而不是裸路径。
        // 所有启动失败路径都必须进入可解释的 Error 并推送 core-status-changed
        // （不得永久停在 Starting / 假运行 / 假停止）。
        if let Some(hint) = self.init_error.clone() {
            let err = Error::NotFound(hint);
            return Err(self.transition_to_error_and_emit(err));
        }

        if !mihomo_path.exists() {
            let hint = crate::util::paths::mihomo_missing_hint(&self.app_handle);
            let err = Error::NotFound(hint);
            return Err(self.transition_to_error_and_emit(err));
        }

        // 1. 回收上次会话遗留的孤儿 mihomo（应用被强杀/崩溃后内核仍占着端口）。
        {
            let (d, m) = (data_dir.clone(), mihomo_path.clone());
            match tokio::task::spawn_blocking(move || spawn::reclaim_orphan(&d, &m)).await {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => return Err(self.transition_to_error_and_emit(e)),
                Err(e) => {
                    return Err(self.transition_to_error_and_emit(Error::Other(format!(
                        "orphan reclaim task failed: {}",
                        e
                    ))))
                }
            }
        }

        // 2. 端口预检：mixed-port / DNS 被占用直接报"谁占用了"；控制器端口被占用则
        //    本次会话改选空闲端口（仅内存生效）。
        {
            // 预检看"配置的"端口（不含上次会话的覆盖），保证每次启动重新判定。
            let (m, cfg) = (mihomo_path.clone(), self.config.read().clone());
            match tokio::task::spawn_blocking(move || spawn::preflight(&m, &cfg)).await {
                Ok(Ok(pre)) => {
                    // 仅会话覆盖；每次启动重新判定（旧覆盖在端口恢复空闲后自然失效）。
                    *self.controller_override.write() = pre.controller_override;
                }
                Ok(Err(e)) => return Err(self.transition_to_error_and_emit(e)),
                Err(e) => {
                    return Err(self.transition_to_error_and_emit(Error::Other(format!(
                        "port preflight task failed: {}",
                        e
                    ))))
                }
            }
        }

        // 3. 用当前配置（含激活 Profile）生成运行时配置。
        //    写盘失败（磁盘只读/空间不足等）绝不能停留在 Starting。
        spawn::ensure_rule_files(&data_dir);
        let config = self.config();
        let (runtime_config, hash) = match spawn::write_runtime_config(&data_dir, &config) {
            Ok(v) => v,
            Err(e) => return Err(self.transition_to_error_and_emit(e)),
        };

        // 4. 启动（日志追加写入；会话记录落盘）。
        let spawned = match spawn::spawn_mihomo(&mihomo_path, &data_dir, &runtime_config) {
            Ok(s) => s,
            Err(e) => return Err(self.transition_to_error_and_emit(e)),
        };
        let stdout_offset = spawned.stdout_offset;
        *self.child.lock().unwrap() = Some(spawned.child);
        // 同步 PID 缓存
        self.record_pid_cache();

        // 5. 就绪探测：轮询 REST /version（mihomo 起不来的话这里会超时 → Error，不假成功）
        match self.wait_ready().await {
            Ok(()) => {
                // 端口监听确认：REST 就绪只代表控制器可达，不代表 mixed-port 已监听。
                // 预检已排除"启动前就被占用"，这里再核对 mihomo 实际应用的端口，
                // 不依赖日志级别（log-level: silent 时日志解析会失效）。
                if let Err(e) = self.wait_mixed_port().await {
                    let _ = self.stop_locked().await; // 清掉僵尸进程
                    return Err(self.transition_to_error_and_emit(e));
                }
                // 日志兜底：只读本会话新增内容中的 bind 错误（如 DNS 监听失败）。
                if let Err(bind_err) = self.detect_bind_conflict(stdout_offset) {
                    let _ = self.stop_locked().await;
                    return Err(self.transition_to_error_and_emit(bind_err));
                }
                *self.status.lock().unwrap() = CoreStatus::Running;
                // 缓存版本（REST 优先；失败回退 `-v`）
                if let Ok(v) = self.version().await {
                    *self.version_cache.lock().unwrap() = Some(v);
                }
                // 新进程启动时刻（稳定运行判定基准）
                *self.started_at.lock().unwrap() = Some(std::time::Instant::now());
                *self.applied_hash.lock().unwrap() = Some(hash);
                // 只 spawn 一个携带当前 generation 的 watcher
                self.spawn_watcher(generation);
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
                // 顺序约束：先清理僵尸进程（stop_locked 可能置 Stopping/Stopped，
                // 也可能因杀不掉置 Error），最后统一以 Error + core-status-changed
                // 收尾——若先置 Error 再 stop_locked()，后者会无条件覆盖成
                // Stopped，UI 呈现"假停止"且丢失失败原因。
                let _ = self.stop_locked().await; // 清掉起不来的僵尸进程
                Err(self.transition_to_error_and_emit(e))
            }
        }
    }

    /// 轮询 REST `/version` 直到就绪或超时；期间若子进程提前退出则立即报错。
    async fn wait_ready(&self) -> Result<()> {
        let url = self.controller.api_url(&["version"], None)?;
        let headers = self.controller.api_headers()?;
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
                .controller
                .api_client()
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

    /// 启动后核对：GET /configs 的 mixed-port 等于期望值，且该端口 TCP 可连接。
    /// mihomo 的监听器与 API 并发启动，允许短暂重试。
    async fn wait_mixed_port(&self) -> Result<()> {
        let expected = self.config.read().general.mixed_port;
        let mut last: Option<Error> = None;
        for _ in 0..8 {
            let live = self.live_mixed_port().await;
            match live {
                Ok(Some(p)) if p == expected => {
                    match probe_tcp(("127.0.0.1", expected), MIXED_PORT_PROBE_TIMEOUT).await {
                        Ok(()) => return Ok(()),
                        Err(e) => last = Some(e),
                    }
                }
                Ok(other) => {
                    last = Some(Error::Other(format!(
                        "live mixed-port is {:?}, expected {}",
                        other, expected
                    )))
                }
                Err(e) => last = Some(e),
            }
            tokio::time::sleep(READY_POLL_INTERVAL).await;
        }
        Err(Error::Other(format!(
            "端口 {} 未能监听：{}",
            expected,
            last.map(|e| e.to_string()).unwrap_or_default()
        )))
    }

    async fn live_mixed_port(&self) -> Result<Option<u16>> {
        let url = self.controller.api_url(&["configs"], None)?;
        let resp = self
            .controller
            .api_client()
            .get(url)
            .headers(self.controller.api_headers()?)
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(Error::Other(format!(
                "GET /configs returned {}",
                resp.status()
            )));
        }
        let live: serde_json::Value = resp.json().await?;
        Ok(live
            .get("mixed-port")
            .and_then(|v| v.as_u64())
            .and_then(|p| u16::try_from(p).ok()))
    }

    /// 停止 mihomo 进程
    pub async fn stop(&self) -> Result<()> {
        let _lifecycle_guard = self.lifecycle.lock().await;
        self.stop_locked().await
    }

    /// stop 实现体（调用方必须已持有 lifecycle 锁，见 start_locked 注释）。
    ///
    /// 停止假成功防护：必须确认进程确实退出才能返回 Ok。`child.kill()` 的
    /// 错误逐一检查；仍未退出时按"持句柄 + 映像路径校验"的 `terminate_owned`
    /// 兜底（不再依赖 taskkill）。杀不掉时**不**置 Stopped，
    /// 而是保留可追踪的 PID 在 `core_pid_cache`（供退出清理继续追踪）并进入
    /// Error + 推送 core-status-changed。绝不按进程名杀进程。
    async fn stop_locked(&self) -> Result<()> {
        // 递增 generation 使旧 watcher 失效（防止它把本次主动停止
        // 当成崩溃处理）；标记用户主动停止双保险
        self.generation
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.user_stopped
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let child = self.child.lock().unwrap().take();
        let Some(mut child) = child else {
            // 无子进程：除非已处于 Error，否则回到 Stopped 并清空缓存。
            let is_error = matches!(*self.status.lock().unwrap(), CoreStatus::Error(_));
            if !is_error {
                *self.status.lock().unwrap() = CoreStatus::Stopped;
            }
            self.clear_pid_cache();
            spawn::clear_session(&self.data_dir);
            return Ok(());
        };

        // 先记下 PID：kill 后未能确认退出时需要按 PID 精确兜底清杀
        let pid = child.id();
        info!("Stopping mihomo (PID {})", pid.unwrap_or(0));
        *self.status.lock().unwrap() = CoreStatus::Stopping;

        // 1. 温和 kill（错误不立即失败——可能进程刚好已退出；交由确认轮询判定）
        let kill_result = child.kill().await;
        if let Err(e) = &kill_result {
            debug!("kill() reported error (may already be gone): {}", e);
        }

        // 2. 确认是否真正退出（轮询 try_wait，不依赖可能被取消的 wait()）
        let mut stopped = self
            .confirm_child_exit(&mut child, Duration::from_secs(3))
            .await;
        let mut failure_reason: Option<String> = None;

        // 3. 未退出：持句柄终止（校验映像路径），并确认进程真消失
        if !stopped {
            match pid {
                Some(pid) => {
                    warn!(
                        "mihomo did not exit after kill; handle-based terminate fallback (PID {})",
                        pid
                    );
                    let path = self.mihomo_path.clone();
                    let res = tokio::task::spawn_blocking(move || {
                        crate::util::process::terminate_owned(pid, &path, 2000)
                    })
                    .await;
                    match res {
                        Ok(Ok(_)) => {
                            stopped = self
                                .confirm_child_exit(&mut child, Duration::from_secs(2))
                                .await;
                            if !stopped {
                                failure_reason = Some(format!(
                                    "terminate PID {} requested but process is still alive",
                                    pid
                                ));
                            }
                        }
                        Ok(Err(e)) => {
                            failure_reason = Some(format!("terminate PID {} failed: {}", pid, e));
                        }
                        Err(e) => {
                            failure_reason =
                                Some(format!("terminate task for PID {} failed: {}", pid, e));
                        }
                    }
                }
                None => {
                    failure_reason =
                        Some("child has no PID and did not exit after kill".to_string());
                }
            }
        }

        if stopped {
            // 进程确认退出后立即清空 PID 缓存，防止 PID 被系统复用后误杀
            self.clear_pid_cache();
            spawn::clear_session(&self.data_dir);
            *self.status.lock().unwrap() = CoreStatus::Stopped;
            info!("mihomo stopped");
            Ok(())
        } else {
            // 停止失败：保留可追踪的 PID（供退出清理兜底），进入 Error 并推送事件
            if let Some(pid) = pid {
                self.set_pid_cache(pid);
            }
            // Keep the handle so a subsequent user-initiated stop can retry the
            // termination instead of clearing the last PID while the process lives.
            *self.child.lock().unwrap() = Some(child);
            let msg = format!(
                "failed to stop mihomo (PID {}): {}",
                pid.map(|p| p.to_string())
                    .unwrap_or_else(|| "?".to_string()),
                failure_reason.unwrap_or_else(|| "unknown reason".to_string())
            );
            error!("{msg}");
            let err = Error::Other(msg);
            Err(self.transition_to_error_and_emit(err))
        }
    }

    /// 重启 mihomo
    pub async fn restart(&self) -> Result<()> {
        let _lifecycle_guard = self.lifecycle.lock().await;
        // tokio Mutex 不可重入：self.stop().await? / self.start().await 会再次
        // lock 同一把锁 → 永久死锁。必须走 *_locked 内部实现体，串行语义不变。
        self.restart_locked().await
    }

    /// restart 实现体（调用方必须已持有 lifecycle 锁，见 start_locked 注释）。
    async fn restart_locked(&self) -> Result<()> {
        self.stop_locked().await?;
        self.start_locked().await
    }

    /// 热重载后的真实运行状态校验。PUT /configs 返回 200 不代表新配置
    /// 真正生效（mihomo 可能静默跳过非法字段/监听失败），必须核对：
    /// 1. `/version` 可达（控制器活着）；
    /// 2. GET /configs 的 mixed-port 与期望一致（关键字段已应用）；
    /// 3. configured mixed-port 实际 TCP 可连接；
    /// 4. dns.enable=true 时对应 listen 端口可连接。
    ///
    /// 任何一项失败都返回 Err，由调用方回滚或回退重启。
    pub async fn verify_runtime_applied(&self) -> Result<()> {
        // 1. /version 可访问
        let url = self.controller.api_url(&["version"], None)?;
        let resp = self
            .controller
            .api_client()
            .get(url)
            .headers(self.controller.api_headers()?)
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(Error::Other(format!(
                "post-reload check: controller /version returned {}",
                resp.status()
            )));
        }

        // 2. GET /configs 核对关键字段（mixed-port）
        let expected_port = self.config.read().general.mixed_port;
        match self.live_mixed_port().await {
            Ok(Some(p)) if p == expected_port => {}
            Ok(other) => {
                return Err(Error::Other(format!(
                    "post-reload check: live mixed-port is {:?}, expected {}",
                    other, expected_port
                )));
            }
            Err(e) => {
                return Err(Error::Other(format!(
                    "post-reload check: GET /configs failed: {}",
                    e
                )));
            }
        }

        // 3. configured mixed-port 实际监听（TCP 可连接）
        probe_tcp(("127.0.0.1", expected_port), MIXED_PORT_PROBE_TIMEOUT).await?;

        // 4. DNS enable=true 时 listen 端口正常（mihomo DNS 监听 TCP/UDP 双栈，
        //    TCP 可连接即视为监听成功）
        let dns = self.config.read().dns.clone();
        if dns.enable && !dns.listen.is_empty() {
            probe_str_addr(&normalize_dns_listen(&dns.listen)).await?;
        }
        Ok(())
    }

    /// 用当前共享配置重新生成 runtime-config.yaml（不重启、不推事件）。
    /// 供编排层（core::runtime）在持久化后调用，保证下次启动/重载即用新值。
    pub fn regen_runtime_config(&self) -> Result<()> {
        let config = self.config();
        self.write_runtime_config(&config).map(|_| ())
    }

    /// 重载配置：重新生成 runtime-config.yaml；若与当前已应用的运行时配置
    /// 内容完全一致（例如只改了语言等应用级字段）则跳过热重载；
    /// 运行中用 REST 热重载，热重载后必须通过健康检查；
    /// REST 失败、校验失败或未运行时回退整进程重启。
    pub async fn reload_config(&self) -> Result<()> {
        self.reload_impl(false).await
    }

    /// 强制重载（用户显式点击"重载配置"、geodata 文件被替换后等必须重新读盘的场景）。
    pub async fn reload_config_force(&self) -> Result<()> {
        self.reload_impl(true).await
    }

    async fn reload_impl(&self, force: bool) -> Result<()> {
        let _lifecycle_guard = self.lifecycle.lock().await;
        let config = self.config();
        let hash = self.write_runtime_config(&config)?;

        if self.is_running() {
            if !force && self.applied_hash.lock().unwrap().as_deref() == Some(hash.as_str()) {
                debug!("Runtime config unchanged (hash {}); skip hot reload", hash);
                return Ok(());
            }
            let yaml = std::fs::read_to_string(self.runtime_config_path())?;
            let payload = serde_json::json!({ "path": "", "payload": yaml });
            let url = self
                .controller
                .api_url(&["configs"], Some(&[("force", "true")]))?;
            let resp = self
                .controller
                .api_client()
                .put(url)
                .headers(self.controller.api_headers()?)
                .json(&payload)
                .send()
                .await;
            match resp {
                Ok(resp) if resp.status().is_success() => {
                    // HTTP 200 不够——健康检查通过才算 reload 成功；
                    // 失败则回退整进程重启（start() 自带就绪 + bind 冲突检测）。
                    match self.verify_runtime_applied().await {
                        Ok(()) => {
                            info!("Runtime config hot-reloaded via PUT /configs (health OK)");
                            *self.applied_hash.lock().unwrap() = Some(hash);
                            // 版本缓存失效，重取
                            if let Ok(v) = self.version().await {
                                *self.version_cache.lock().unwrap() = Some(v);
                            }
                            return Ok(());
                        }
                        Err(e) => {
                            warn!(
                                "Hot-reload health check failed ({}); falling back to restart",
                                e
                            );
                        }
                    }
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
        // reload 已持 lifecycle 锁，直接调 restart_locked
        // 而非 restart()（后者会再 lock → 死锁）。
        self.restart_locked().await
    }
}
