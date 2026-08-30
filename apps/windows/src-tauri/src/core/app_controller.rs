// src-tauri/src/core/app_controller.rs
//! 应用事务控制器（AppController）
//!
//! 配置/运行态事务串行锁的唯一持有者。所有改变系统运行态的入口
//! （update_config / update_config_fields / reset_config / import_config /
//! apply_proxy_mode / apply_tun / apply_system_proxy / activate_profile /
//! rename_profile / update_profile_content / 订阅提交 / tray config_mixin /
//! stop_core）都必须经由本控制器，事务锁的获取发生在控制器方法内部，
//! 调用方**不可能**忘记持锁或绕过它。
//!
//! 事务主体（`*_locked` 函数）保留在原处：
//! - apply_* / activate_profile 主体在 `core::runtime`；
//! - 整包/字段级配置事务主体在本文件（自 `commands/config.rs` 收拢）；
//! - profile 文件 + 激活复合事务主体在 `commands::profiles`。
//!
//! 各 mutating 方法在取锁前完成所有"只读/慢速网络"预检（降级守卫、
//! 代理模式合法性、TUN 管理员权限、ensure_core_serving、订阅下载与校验），
//! 取锁后执行「持久化 → 重新生成运行时配置 → PATCH/重启 mihomo →
//! 失败回滚 → 通知」链路，与既往行为的锁持有时序完全一致。

use std::path::Path;

use tauri::{AppHandle, Emitter, Manager};
use tracing::error;

use crate::config::model::Config;
use crate::proxy::system_proxy::SystemProxyConfig;
use crate::util::error::{Error, Result};

/// 应用事务控制器：持有配置/运行态事务串行锁（tokio Mutex，可跨 `.await`）。
pub struct AppController {
    /// 事务串行锁（原 `AppState.config_tx`，语义不变）。锁的是 `()`——
    /// 纯串行作用，不承载任何数据。所有改变 Config + Mihomo + Windows 的
    /// 入口必须在做事之前经本控制器取锁并持有到事务结束，保证
    /// 「UI = Config = runtime-config = Mihomo = Windows」在并发入口下
    /// 也严格成立。
    tx: tokio::sync::Mutex<()>,
}

impl Default for AppController {
    fn default() -> Self {
        Self::new()
    }
}

impl AppController {
    pub fn new() -> Self {
        Self {
            tx: tokio::sync::Mutex::new(()),
        }
    }

    /// 应用代理模式：校验 → 持久化 → 同步运行时 → PATCH 运行中核心 → 失败回滚。
    ///
    /// 合法性校验在取事务锁之前完成（非法模式不占用事务锁，与原实现一致）。
    pub async fn apply_proxy_mode(&self, app: &AppHandle, mode: &str) -> Result<()> {
        crate::core::runtime::validate_proxy_mode(mode)?;
        let _tx = self.tx.lock().await;
        crate::core::runtime::apply_proxy_mode_locked(app, mode).await
    }

    /// 应用 TUN 开关：持久化 → 同步运行时 → PATCH 运行中核心 → 确认实际状态。
    ///
    /// 「确认实际结果」是本流程的核心：PATCH /configs 返回 200 不代表 mihomo
    /// 真正接受并运行了目标 TUN 状态（可能静默跳过非法字段 / 内核未能建立网卡）。
    /// PATCH 后会回读运行中核心的 `tun.enable`，与目标值比对。
    ///
    /// 权限预检（开启 TUN 需要管理员权限）在取事务锁之前完成，与原实现一致。
    pub async fn apply_tun(&self, app: &AppHandle, enable: bool) -> Result<()> {
        crate::core::runtime::validate_tun_permission(enable)?;
        let _tx = self.tx.lock().await;
        crate::core::runtime::apply_tun_locked(app, enable).await
    }

    /// 应用系统代理：持久化用户意图 → 写 Windows 注册表（真实生效）→ 失败回滚。
    ///
    /// 开启前必须确认 Core Running 且 mixed-port 实际 TCP 可连接（ensure_core_serving），
    /// 此校验在任何持久化之前、且在取事务锁**之前**执行——它可能触发一次慢速
    /// start()/restart()（含就绪轮询，最长 ~10s），若在锁内执行会长期占住全局
    /// 事务锁，导致其余开关（TUN/代理模式/mixin/托盘）全部排队等待。
    pub async fn apply_system_proxy(&self, app: &AppHandle, enable: bool) -> Result<()> {
        // 开启前必须确认核心在服务（不满足时先尝试自动启动核心，仍失败则拒绝
        // 开启并返回明确错误——绝不能让 Windows 指向无人监听的代理端口）。
        // 失败时不留下任何半套状态。
        if enable {
            crate::core::runtime::ensure_core_serving(app).await?;
        }

        // 全程持有事务锁，串行整段事务。
        let _tx = self.tx.lock().await;
        crate::core::runtime::apply_system_proxy_locked(app, enable).await
    }

    /// 停止核心并同步系统代理（统一编排入口）。
    /// 所有停止核心的调用都必须经由本方法，避免绕过配置事务或
    /// 与 apply_* 编排层形成两套路径。
    ///
    /// 执行顺序（网络安全优先）：先退出系统代理接管，再停止核心。
    /// 若先停核心再关系统代理，关系统代理的 set_config 一旦失败，会留下
    /// "Windows 代理仍指向已死的 127.0.0.1:7890" 的断网状态。反过来：即使
    /// 退系统代理失败，也不停止核心，用户至少保持可上网。
    pub async fn stop_core_and_sync_proxy(&self, app: &AppHandle) -> Result<()> {
        // 1) 先退出系统代理接管（config/registry/journal/事件统一事务）。
        //    失败则直接返回，不停止核心——宁可保持核心运行也不掐断用户网络。
        self.apply_system_proxy(app, false).await?;

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
        crate::core::runtime::refresh_tray(app).await?;
        Ok(())
    }

    /// 激活 Profile：校验名称合法且文件存在 → 持久化激活名 → 重新生成运行时配置 →
    /// 热重载运行中的核心 → 失败回滚。空内容的 Profile 不阻塞：build_runtime_config
    /// 会回退到内置模板。
    pub async fn activate_profile(&self, app: &AppHandle, name: &str) -> Result<()> {
        let _tx = self.tx.lock().await;
        crate::core::runtime::activate_profile_locked(app, name).await
    }

    /// 整包配置事务：校验已完成，这里执行
    /// 「持久化 → 应用运行时（含健康检查）→ Windows 副作用 → commit；
    ///   任一步失败 → Config / runtime-config / Mihomo / Windows 全部回滚」。
    pub async fn update_config(&self, app: &AppHandle, new_config: Config) -> Result<()> {
        let _tx = self.tx.lock().await;
        let state = app.state::<crate::AppState>();
        self.commit_config_locked(app, &state, new_config).await
    }

    /// 字段级配置事务：先持有事务锁，再读取最新配置合并 patch 并校验，
    /// 然后走与整包事务完全相同的持久化/运行时/回滚流程。
    ///
    /// 并发正确性：读取+合并必须发生在持锁之后——若在加锁前读取并合并当前配置，
    /// 另一个事务可能在"读取后、加锁前"完成提交，随后被本事务的旧快照整包覆盖。
    /// 在持锁后重新读取最新配置再合并，保证不同字段的并发更新互不覆盖。
    pub async fn update_config_fields(
        &self,
        app: &AppHandle,
        patch: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<()> {
        // 先拿全局配置事务锁，保证"读取最新配置"与"提交"之间没有其他事务插入。
        let _tx = self.tx.lock().await;
        let state = app.state::<crate::AppState>();
        let new_config = {
            let config_guard = state.config_manager.lock().unwrap();
            let mut current = serde_json::to_value(config_guard.get_config())?;
            let cur_obj = current
                .as_object_mut()
                .ok_or_else(|| Error::Other("当前配置不是 JSON 对象".to_string()))?;
            for (k, v) in patch {
                cur_obj.insert(k.clone(), v.clone());
            }
            config_guard.prepare_update(current)?
        };
        self.commit_config_locked(app, &state, new_config).await
    }

    /// 切换托盘「配置覆写 mixin」开关：翻转配置并持久化 → 刷新托盘 → 通知前端。
    ///
    /// mixin_enabled 是应用级字段（不影响 runtime-config.yaml），
    /// 切换不需要 reload mihomo，但仍要持事务锁串行，避免与
    /// update_config / apply_* 等并发事务在 config_manager
    /// 上交错（否则可能撞上正在 reload 的事务拿到中间态配置）。
    pub async fn toggle_config_mixin(&self, app: &AppHandle) -> Result<bool> {
        let _tx = self.tx.lock().await;
        let state = app.state::<crate::AppState>();
        let new_val = {
            let mut cfg = state.config_manager.lock().unwrap();
            let mut c = cfg.get_config();
            c.mixin_enabled = !c.mixin_enabled;
            let v = c.mixin_enabled;
            cfg.set_config(c)?;
            v
        };
        // 刷新托盘菜单勾选态，并通知前端同步 UI 状态。
        crate::core::runtime::refresh_tray(app).await?;
        let _ = app.emit(
            "config-mixin-changed",
            serde_json::json!({ "enable": new_val }),
        );
        Ok(new_val)
    }

    /// 重命名 Profile 复合事务（前置校验已由命令完成）：rename 文件 →
    /// 激活新名（持久化 config.profile + 重生成运行时 + 重启核心）→ 失败回滚。
    pub async fn rename_profile(
        &self,
        app: &AppHandle,
        old_name: &str,
        new_name: &str,
        old_path: &Path,
        new_path: &Path,
    ) -> Result<()> {
        let _tx = self.tx.lock().await;
        crate::commands::profiles::rename_profile_locked(
            app, old_name, new_name, old_path, new_path,
        )
        .await
    }

    /// 保存 Profile 内容复合事务（前置校验已由命令完成）：写入临时文件 →
    /// 事务式替换正式文件 → 激活中的 Profile 立即生效（失败回滚 .bak 旧内容）。
    pub async fn update_profile_content(
        &self,
        app: &AppHandle,
        name: &str,
        file_path: &Path,
        content: &str,
    ) -> Result<()> {
        let _tx = self.tx.lock().await;
        crate::commands::profiles::update_profile_content_locked(app, name, file_path, content)
            .await
    }

    /// 订阅更新提交复合事务（下载/校验/归一化已由调用方在锁外完成）：
    /// 覆写临时文件 → 事务式替换正式文件 → 激活中的 Profile 热重载生效。
    pub async fn commit_subscription_update(
        &self,
        app: &AppHandle,
        name: &str,
        file_path: &Path,
        temp_path: &Path,
        final_text: &str,
    ) -> Result<()> {
        let _tx = self.tx.lock().await;
        crate::commands::profiles::commit_subscription_update_locked(
            app, name, file_path, temp_path, final_text,
        )
        .await
    }

    /// 整包配置事务主体（调用方必须已持有事务锁）。字段级入口
    /// （`update_config_fields`）在锁内重读最新配置后复用本方法。
    async fn commit_config_locked(
        &self,
        app: &AppHandle,
        state: &tauri::State<'_, crate::AppState>,
        mut new_config: Config,
    ) -> Result<()> {
        // 1. 快照旧配置（回滚基准）
        let old = { state.config_manager.lock().unwrap().get_config() };

        // mixed-port 变化或关闭系统代理前，必须先释放旧 ownership 并确认用户基线，
        // 再停止/重启旧端口的 Mihomo。否则 runtime 先切到新端口的数秒内，Windows
        // 仍指向已经无人监听的旧端口，会制造短暂断网。
        let proxy_transition = prepare_proxy_transition(app, &old, &new_config)?;
        if matches!(proxy_transition, ProxyTransition::OwnershipUnavailable) {
            // 用户/其他软件已经接管：保留 Windows 状态，并取消新配置的自动接管意图。
            new_config.general.system_proxy = false;
        }

        // 2. 持久化新配置（disk-first：落盘成功才提交内存）
        {
            let mut guard = state.config_manager.lock().unwrap();
            guard.set_config(new_config)?;
        }

        // 3. 应用到运行时：重写 runtime-config.yaml + 热重载/重启运行中的核心。
        //    reload 成功与否由真实运行状态健康检查决定，不以 HTTP 200 为准。
        //    核心未运行时 reload_running_core 只重写文件，不会失败于此路径之外。
        if let Err(e) = reload_running_core(state).await {
            error!("Config change failed to apply ({}); rolling back", e);

            // 4a. 回滚持久化 + 运行时（内存 + 磁盘恢复旧值，再拉回旧运行态）
            if let Err(rb) =
                rollback_config_runtime_and_proxy(app, state, old, &proxy_transition).await
            {
                return Err(Error::Other(format!(
                    "配置应用失败（{}），且配置回滚也失败：{}",
                    e, rb
                )));
            }
            return Err(Error::Other(format!("配置已保存但应用失败，已回滚：{}", e)));
        }

        // 5. Windows 副作用同步——注册表必须与新配置意图一致。
        //    失败则完整回滚四层状态，禁止出现「Config=new / runtime=new / Windows=old」。
        if let Err(e) = sync_windows_side_effects(app, state, &proxy_transition).await {
            error!(
                "Config change applied but Windows side-effect failed ({}); rolling back fully",
                e
            );

            // 新端口 ownership 只有在能安全释放/确认后，才允许回滚 runtime；
            // 若当前仍指向新端口但字段已被并发修改，必须保留新核心避免死代理。
            let attempted = { state.config_manager.lock().unwrap().get_config() };
            let data_dir = crate::util::paths::get_app_data_dir(app)?;
            let unwind = if attempted.general.system_proxy {
                match crate::proxy::journal::release_owned_proxy(
                    &data_dir,
                    attempted.general.mixed_port,
                ) {
                    Ok(crate::proxy::journal::ReleaseOutcome::Restored { restored, .. }) => {
                        ProxyTransition::Released(restored)
                    }
                    Ok(crate::proxy::journal::ReleaseOutcome::OwnershipLost)
                    | Ok(crate::proxy::journal::ReleaseOutcome::NoOwnership) => {
                        ProxyTransition::OwnershipUnavailable
                    }
                    Err(release_error) => {
                        return Err(Error::Other(format!(
                            "系统代理同步失败（{}），且无法安全释放新端口 ownership（{}）；为避免死代理，保留当前 runtime/Mihomo 与 journal",
                            e, release_error
                        )));
                    }
                }
            } else {
                proxy_transition.clone()
            };

            let rb_err = match rollback_config_runtime_and_proxy(app, state, old, &unwind).await {
                Ok(()) => None,
                Err(rb) => {
                    error!(
                        "Config rollback during Windows side-effect failure also failed: {}",
                        rb
                    );
                    Some(rb)
                }
            };
            return Err(match rb_err {
                Some(rb) => Error::Other(format!(
                    "系统代理同步失败（{}），已回滚；但配置回滚也失败：{}",
                    e, rb
                )),
                None => Error::Other(format!("系统代理同步失败，已完整回滚：{}", e)),
            });
        }

        Ok(())
    }
}

/// 回滚持久化（内存 + 磁盘恢复旧值）并尽力把 Mihomo 运行时拉回旧配置。
/// 持久化回滚失败 → Err；运行时恢复失败 → 记录后返回该错误（不掩盖原始错误）。
async fn rollback_config_and_runtime(
    state: &tauri::State<'_, crate::AppState>,
    old: Config,
) -> Result<()> {
    {
        let mut guard = state.config_manager.lock().unwrap();
        guard.set_config(old)?;
    }
    if let Err(rb) = reload_running_core(state).await {
        tracing::warn!("Rollback runtime restore failed: {}", rb);
        return Err(rb);
    }
    Ok(())
}

#[derive(Debug, Clone)]
enum ProxyTransition {
    /// 本次配置变更不需要提前释放旧代理。
    None,
    /// 已确认释放旧 ownership，Windows 当前应精确等于该用户基线。
    Released(SystemProxyConfig),
    /// ownership 已由用户/其他软件拿走，或缺少可用凭据；不得重新接管。
    OwnershipUnavailable,
}

fn prepare_proxy_transition(
    app: &AppHandle,
    old: &Config,
    new: &Config,
) -> Result<ProxyTransition> {
    let must_release = old.general.system_proxy
        && (!new.general.system_proxy || old.general.mixed_port != new.general.mixed_port);
    if !must_release {
        return Ok(ProxyTransition::None);
    }
    let data_dir = crate::util::paths::get_app_data_dir(app)?;
    match crate::proxy::journal::release_owned_proxy(&data_dir, old.general.mixed_port)? {
        crate::proxy::journal::ReleaseOutcome::Restored { restored, .. } => {
            Ok(ProxyTransition::Released(restored))
        }
        crate::proxy::journal::ReleaseOutcome::OwnershipLost
        | crate::proxy::journal::ReleaseOutcome::NoOwnership => {
            Ok(ProxyTransition::OwnershipUnavailable)
        }
    }
}

/// 配置/runtime 回滚后，仅凭本事务刚确认的用户基线重新接管旧端口。
/// ownership 已丢失时把旧配置意图落回 false，绝不覆盖外部代理。
async fn rollback_config_runtime_and_proxy(
    app: &AppHandle,
    state: &tauri::State<'_, crate::AppState>,
    mut old: Config,
    transition: &ProxyTransition,
) -> Result<()> {
    if matches!(transition, ProxyTransition::OwnershipUnavailable) {
        old.general.system_proxy = false;
    }
    let old_port = old.general.mixed_port;
    let old_proxy_intent = old.general.system_proxy;
    rollback_config_and_runtime(state, old).await?;

    if old_proxy_intent {
        if let ProxyTransition::Released(expected_baseline) = transition {
            crate::core::runtime::ensure_core_serving(app).await?;
            let data_dir = crate::util::paths::get_app_data_dir(app)?;
            if let Err(e) = crate::proxy::journal::acquire_system_proxy_if_unchanged(
                &data_dir,
                old_port,
                Some(expected_baseline),
            ) {
                crate::core::runtime::mark_system_proxy_failed(app, &e.to_string()).await;
                return Err(Error::Other(format!(
                    "旧端口 runtime 已恢复，但系统代理 ownership 无法安全恢复：{}",
                    e
                )));
            }
        }
    }
    Ok(())
}

/// 让 Windows 注册表与新配置的 system-proxy 意图一致。
///
/// - 新配置开启系统代理：先确保 Core Running 且 mixed-port 真实可连
///   （不开死代理），再把注册表指向 `127.0.0.1:<新 mixed-port>` 并维护 journal；
/// - 新配置关闭系统代理（import/reset 可能改变它）：把注册表还原为 journal /
///   快照记录的用户原始代理状态（无则关闭），并清除 journal。
async fn sync_windows_side_effects(
    app: &AppHandle,
    state: &tauri::State<'_, crate::AppState>,
    transition: &ProxyTransition,
) -> Result<()> {
    use tracing::info;

    let cfg = { state.config_manager.lock().unwrap().get_config() };
    let data_dir = crate::util::paths::get_app_data_dir(app)?;

    if cfg.general.system_proxy {
        // 开启（或 mixed-port 变更后重新指向）：先保证核心真实服务新端口，
        // 再通过统一 ownership helper 安全释放旧端口并接管新端口。
        crate::core::runtime::ensure_core_serving(app).await?;
        match transition {
            ProxyTransition::Released(expected_baseline) => {
                crate::proxy::journal::acquire_system_proxy_if_unchanged(
                    &data_dir,
                    cfg.general.mixed_port,
                    Some(expected_baseline),
                )?;
            }
            ProxyTransition::OwnershipUnavailable => {
                return Err(Error::Other(
                    "Windows proxy ownership changed; refusing to overwrite the new owner"
                        .to_string(),
                ));
            }
            ProxyTransition::None => {
                crate::proxy::journal::acquire_system_proxy(&data_dir, cfg.general.mixed_port)?;
            }
        }
        info!(
            "Windows system proxy synced to 127.0.0.1:{}",
            cfg.general.mixed_port
        );
        Ok(())
    } else {
        // old=true 的关闭路径已在 runtime 切换前释放；old=false 时本来就未接管。
        // 此处不再额外写注册表，避免把没有 ownership 的外部代理当作关闭目标。
        info!("Windows system proxy synced OFF per config intent");
        Ok(())
    }
}

/// 重建运行时配置并对运行中的核心生效（热重载，失败回退整进程重启）。
///
/// 错误必须向上传播：吞掉 reload 失败会留下「新配置已写盘但 Mihomo 仍用
/// 旧值」的假成功。核心未运行时不报错：
/// 文件已重写，下次启动自然加载新配置。
async fn reload_running_core(state: &tauri::State<'_, crate::AppState>) -> Result<()> {
    let core_guard = state.core_manager.get();
    if let Some(core) = core_guard.as_ref() {
        core.reload_config().await?;
    }
    Ok(())
}
