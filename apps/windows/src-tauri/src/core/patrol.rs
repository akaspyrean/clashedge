// src-tauri/src/core/patrol.rs
//! 节点池后台巡检（P0-3）。
//!
//! `trigger_auto_recovery` 的后端能力（订阅 + 地理数据刷新、10 分钟限流、
//! 开关约束）此前只有前端手动测速一个入口——掉线后系统只能干等用户操作。
//! 本模块补上"无人值守"的触发器：每 [`PATROL_INTERVAL`] 读取一次 mihomo
//! `/proxies` 的真实状态，节点池整体失活（已测节点死亡率 ≥
//! [`POOL_DEAD_RATIO`]，或全部路由组的当前生效节点都已死亡）时，调用与
//! 手动触发完全相同的恢复执行体（`commands::update_status::run_auto_recovery`）。
//!
//! 设计约束：
//! - **巡检本身零探测流量**：只消费 mihomo 已有的测速历史（`history`
//!   最后一条 `delay == 0` 即失败；未测过不算死亡），不给上游加压；
//! - 判定逻辑集中在纯函数 [`analyze`]（输入 JSON、输出 [`PoolReport`]），
//!   便于用 fixture 单测覆盖各临界场景；
//! - 恢复执行体自带 10 分钟限流与用户开关约束，巡检高频轮询
//!   不会放大上游压力；
//! - 内核未运行 / 控制器暂不可达视为常态（静默跳过），巡检永不告警刷屏。

use serde_json::Value as Json;
use tauri::Manager;
use tracing::{debug, info, warn};

use crate::util::error::Result;

/// 巡检间隔：60s 一轮。失活判定依赖 mihomo 的测速历史（自动组每 120s
/// 一轮 + max-failed-times=2 被动触发），60s 轮询足够及时且开销可忽略。
const PATROL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

/// 参与失活率统计的最少已测节点数（不足时不按比例判定，避免小池误报）。
const MIN_TESTED_NODES: usize = 3;

/// 已测节点死亡率达到该比例视为"节点池整体失活"。
const POOL_DEAD_RATIO: f64 = 0.8;

/// 判定"当前选路全部失效"至少需要的可评估路由组数。
const MIN_ROUTING_GROUPS: usize = 2;

/// 不参与节点池统计的内置名。
const BUILTIN_NON_NODES: &[&str] = &["DIRECT", "REJECT", "REJECT-DROP", "PASS", "COMPATIBLE"];

/// 巡检主循环（应用启动时 spawn 一次，随进程退出）。
pub async fn run(app: tauri::AppHandle) {
    if let Err(e) = patrol_once(&app).await {
        debug!("Initial node pool patrol skipped: {}", e);
    }
    loop {
        tokio::time::sleep(PATROL_INTERVAL).await;
        if let Err(e) = patrol_once(&app).await {
            debug!("Node pool patrol skipped: {}", e);
        }
    }
}

/// 单轮巡检：内核在跑 → 取 /proxies → 失活判定 → 必要时触发自动恢复。
async fn patrol_once(app: &tauri::AppHandle) -> Result<()> {
    let state = app.state::<crate::AppState>();
    let Some(failover) = state.profile_failover.get() else {
        return Ok(());
    };
    // Reconcile persisted profile state even when Mihomo failed to start.
    state.controller.reconcile_profile_failover(app).await?;
    // Capture the profile before reading its proxy report. If a user changes
    // profiles while the report is collected, backup activation must be skipped.
    let report_profile = state
        .config_manager
        .lock()
        .unwrap()
        .get_config()
        .general
        .profile;
    let proxies = {
        let Some(core) = state.core_manager.get() else {
            return Ok(()); // 内核尚未初始化（应用启动早期）
        };
        if !core.is_running() {
            return Ok(()); // 内核未运行：巡检静默跳过
        }
        core.get_proxies_raw().await?
    };
    let report = analyze(&proxies);

    if let Some(active) = failover.active() {
        let current_profile = state
            .config_manager
            .lock()
            .unwrap()
            .get_config()
            .general
            .profile;
        let interrupted_probe =
            current_profile == active.primary_profile && failover.probe_in_progress();
        if current_profile != active.backup_profile && !interrupted_probe {
            // 用户手动切换了 Profile；持久化抑制标记，避免下一轮重新切回备份。
            state.controller.reconcile_profile_failover(app).await?;
        } else if profile_file_exists(app, &active.primary_profile) {
            match state
                .controller
                .probe_primary_for_failback(app, &active.primary_profile, &active.backup_profile)
                .await
            {
                Ok(true) => {
                    failover.clear_active()?;
                    info!(
                        "Primary profile '{}' recovered; automatic failback completed",
                        active.primary_profile
                    );
                }
                Ok(false) => {
                    info!(
                        "Primary profile '{}' is still unhealthy; backup '{}' restored",
                        active.primary_profile, active.backup_profile
                    );
                }
                Err(error) => warn!(
                    "Primary recovery probe failed; backup restore was attempted: {}",
                    error
                ),
            }
        }
    } else {
        let primary = state
            .config_manager
            .lock()
            .unwrap()
            .get_config()
            .general
            .profile;
        let manually_overridden = failover.is_manually_overridden(&primary)?;
        // Only use a report captured for the still-active profile, and only an
        // explicitly selected, still-existing backup can be activated.
        if report_profile == primary && report.should_recover() && !manually_overridden {
            if let Some(backup) = failover.backup_profile() {
                if !primary.trim().is_empty()
                    && primary != backup
                    && profile_file_exists(app, &primary)
                    && profile_file_exists(app, &backup)
                {
                    match state
                        .controller
                        .activate_backup_for_failover(app, &primary, &backup)
                        .await
                    {
                        Ok(true) => info!(
                            "Node pool degraded; switched from '{}' to configured backup '{}'",
                            primary, backup
                        ),
                        Ok(false) => debug!(
                            "Primary profile changed during patrol; backup activation skipped"
                        ),
                        Err(error) => {
                            warn!("Unable to activate configured backup profile: {}", error)
                        }
                    }
                }
            }
        }
    }

    let recovery_report = if let Some(core) = state.core_manager.get() {
        if core.is_running() {
            analyze(&core.get_proxies_raw().await?)
        } else {
            report
        }
    } else {
        report
    };
    if recovery_report.should_recover() {
        warn!(
            "Node pool degraded (tested={}, dead={}, ratio={:.2}, routing groups broken {}/{}); \
             triggering auto recovery",
            recovery_report.tested,
            recovery_report.dead,
            recovery_report.dead_ratio,
            recovery_report.routing_broken,
            recovery_report.routing_total
        );
        let outcome = crate::commands::update_status::run_auto_recovery(app).await?;
        info!("Auto recovery outcome: {}", outcome);
    }
    Ok(())
}

fn profile_file_exists(app: &tauri::AppHandle, name: &str) -> bool {
    let Ok(safe_name) = crate::util::paths::sanitize_profile_name(name) else {
        return false;
    };
    let Ok(profiles_dir) = crate::util::paths::get_profiles_dir(app) else {
        return false;
    };
    profiles_dir.join(format!("{}.yaml", safe_name)).is_file()
}

/// 一次巡检的节点池体检结果（全部为可观测计数，便于断言）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PoolReport {
    /// 已测速节点数（history 非空；组 / DIRECT / REJECT 不计入）。
    pub tested: usize,
    /// 其中最近一次测速失败（delay == 0）的节点数。
    pub dead: usize,
    /// dead / tested（tested 不足 [`MIN_TESTED_NODES`] 时为 0.0）。
    pub dead_ratio: f64,
    /// 可评估的路由组数（now 链可解析到已测叶子节点）。
    pub routing_total: usize,
    /// 其中当前生效节点已死的组数。
    pub routing_broken: usize,
}

impl PoolReport {
    /// 是否应触发自动恢复：
    /// - **节点池整体失活**：已测 ≥ [`MIN_TESTED_NODES`] 且死亡率 ≥
    ///   [`POOL_DEAD_RATIO`]（典型：订阅过期 / 机场故障——刷新订阅是唯一出路）；
    /// - **当前选路全部失效**：≥ [`MIN_ROUTING_GROUPS`] 个可评估路由组的
    ///   生效节点全部死亡（典型：生效节点被单独掐死，且各组尚未走到
    ///   下一个探测周期 / url-test 切换尚未完成）。
    pub fn should_recover(&self) -> bool {
        (self.tested >= MIN_TESTED_NODES && self.dead_ratio >= POOL_DEAD_RATIO)
            || (self.routing_total >= MIN_ROUTING_GROUPS
                && self.routing_broken == self.routing_total)
    }
}

/// 值是否为代理组（Selector / URLTest / Fallback / LoadBalance / Relay，
/// 与 mihomo 返回的大小写无关）。
fn is_group_value(v: &Json) -> bool {
    v.get("type").and_then(|t| t.as_str()).is_some_and(|t| {
        ["Selector", "URLTest", "Fallback", "LoadBalance", "Relay"]
            .iter()
            .any(|g| g.eq_ignore_ascii_case(t))
    })
}

/// 节点死亡判定：最近一次测速 delay == 0（mihomo 以 0 记录超时 / 失败）。
/// history 为空 = 从未测过，返回 None（不参与判定，新导入订阅的正常状态）。
fn last_delay(v: &Json) -> Option<u64> {
    v.get("history")
        .and_then(|h| h.as_array())
        .and_then(|arr| arr.last())
        .and_then(|e| e.get("delay"))
        .and_then(|d| d.as_u64())
}

/// 路由组生效节点解析：`now` 可能指向子组（扶梯出行 → 自动优选 → 节点），
/// 沿链最多追 3 层到叶子节点；返回叶子的名字（不要求已测过）。
fn resolve_effective_now<'a>(
    map: &'a serde_json::Map<String, Json>,
    group: &str,
) -> Option<&'a str> {
    let mut cur: &str = group;
    for _ in 0..3 {
        let v = map.get(cur)?;
        let now = v.get("now")?.as_str()?;
        match map.get(now) {
            Some(nv) if is_group_value(nv) => cur = now,
            _ => return Some(now),
        }
    }
    None
}

/// 从 mihomo `GET /proxies` 原始 JSON 提取体检结果（纯函数，便于 fixture 测试）。
pub(crate) fn analyze(proxies: &Json) -> PoolReport {
    let empty = PoolReport {
        tested: 0,
        dead: 0,
        dead_ratio: 0.0,
        routing_total: 0,
        routing_broken: 0,
    };
    let Some(map) = proxies.get("proxies").and_then(|v| v.as_object()) else {
        return empty;
    };

    let mut tested = 0usize;
    let mut dead = 0usize;
    // 节点名 → 是否已死（仅已测节点入表；路由组解析用它判定生效节点生死）
    let mut dead_names: std::collections::HashMap<&str, bool> = std::collections::HashMap::new();
    for (name, v) in map {
        if is_group_value(v) || BUILTIN_NON_NODES.contains(&name.as_str()) {
            continue;
        }
        match last_delay(v) {
            Some(0) => {
                tested += 1;
                dead += 1;
                dead_names.insert(name.as_str(), true);
            }
            Some(_) => {
                tested += 1;
                dead_names.insert(name.as_str(), false);
            }
            None => {}
        }
    }

    // 三个路由策略组 + 自动优选。GLOBAL 是模式容器不参与（rule 模式下
    // 不承载流量；人工优选是叶子选择组，其成员与路由组 now 链重叠）。
    let routing_names = [
        crate::config::model::product_data::GROUP_LADDER,
        crate::config::model::product_data::GROUP_AI,
        crate::config::model::product_data::GROUP_MEDIA,
        crate::config::model::product_data::GROUP_AUTO,
    ];
    let mut routing_total = 0usize;
    let mut routing_broken = 0usize;
    for g in routing_names {
        if !map.contains_key(g) {
            continue;
        }
        let Some(leaf) = resolve_effective_now(map, g) else {
            continue;
        };
        let Some(&is_dead) = dead_names.get(leaf) else {
            continue; // 生效节点未测过：不可评估，不计入
        };
        routing_total += 1;
        if is_dead {
            routing_broken += 1;
        }
    }

    let dead_ratio = if tested >= MIN_TESTED_NODES {
        dead as f64 / tested as f64
    } else {
        0.0
    };
    PoolReport {
        tested,
        dead,
        dead_ratio,
        routing_total,
        routing_broken,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造 /proxies 形状的 fixture：history 最后一条 delay 决定节点生死。
    fn fixture(node_delays: &[(&str, u64)]) -> Json {
        let mut proxies = serde_json::Map::new();
        proxies.insert(
            "DIRECT".to_string(),
            serde_json::json!({ "type": "Direct", "now": "", "all": [], "history": [] }),
        );
        proxies.insert(
            "REJECT".to_string(),
            serde_json::json!({ "type": "Reject", "history": [] }),
        );
        for (name, delay) in node_delays {
            proxies.insert(
                name.to_string(),
                serde_json::json!({
                    "type": "Shadowsocks",
                    "alive": *delay > 0,
                    "history": [{ "time": "2026-11-12T00:00:00Z", "delay": delay }],
                }),
            );
        }
        let first = node_delays.first().map(|(n, _)| *n).unwrap_or("DIRECT");
        let second = node_delays.get(1).map(|(n, _)| *n).unwrap_or(first);
        // 模拟 url-test 真实行为：now 指向第一个存活节点；
        // 全部死亡时指向首节点（死节点霸占的盲区窗口）。
        let auto_now = node_delays
            .iter()
            .find(|(_, d)| *d > 0)
            .map(|(n, _)| *n)
            .unwrap_or(first);
        proxies.insert(
            "自动优选".to_string(),
            serde_json::json!({
                "type": "URLTest", "now": auto_now,
                "all": node_delays.iter().map(|(n, _)| n).collect::<Vec<_>>(),
            }),
        );
        proxies.insert(
            "人工优选".to_string(),
            serde_json::json!({ "type": "Selector", "now": second, "all": [] }),
        );
        proxies.insert(
            "扶梯出行".to_string(),
            serde_json::json!({ "type": "Selector", "now": "自动优选", "all": [] }),
        );
        proxies.insert(
            "人工智能".to_string(),
            serde_json::json!({ "type": "Selector", "now": "自动优选", "all": [] }),
        );
        serde_json::json!({ "proxies": proxies })
    }

    #[test]
    fn healthy_pool_never_recovers() {
        // 3 个已测节点全部存活：比例与路由组两条路径都不触发。
        let r = analyze(&fixture(&[("A", 120), ("B", 180), ("C", 90)]));
        assert_eq!(r.tested, 3);
        assert_eq!(r.dead, 0);
        assert!(!r.should_recover());
    }

    #[test]
    fn untested_pool_is_not_dead() {
        // history 为空（刚导入未测速）：不算死亡，绝不触发。
        let mut proxies = fixture(&[]);
        let obj = proxies.get_mut("proxies").unwrap().as_object_mut().unwrap();
        obj.insert(
            "NodeA".to_string(),
            serde_json::json!({ "type": "Shadowsocks", "history": [] }),
        );
        let r = analyze(&proxies);
        assert_eq!(r.tested, 0);
        assert_eq!(r.dead_ratio, 0.0);
        assert!(!r.should_recover());
    }

    #[test]
    fn pool_death_ratio_triggers_recovery() {
        // 5 测 4 死（80%）：典型订阅过期形态，比例路径必须触发。
        // （自动优选已指向唯一的存活节点，路由组路径不触发——验证比例路径独立生效。）
        let r = analyze(&fixture(&[
            ("A", 0),
            ("B", 0),
            ("C", 0),
            ("D", 0),
            ("E", 150),
        ]));
        assert_eq!(r.tested, 5);
        assert_eq!(r.dead, 4);
        assert!(r.dead_ratio >= 0.8);
        assert!(r.should_recover());
    }

    #[test]
    fn ratio_below_threshold_does_not_trigger() {
        // 10 测 7 死（70%）：低于阈值；且自动优选指向存活节点，不触发。
        let r = analyze(&fixture(&[
            ("A", 0),
            ("B", 0),
            ("C", 0),
            ("D", 0),
            ("E", 0),
            ("F", 0),
            ("G", 0),
            ("H", 100),
            ("I", 120),
            ("J", 90),
        ]));
        assert_eq!(r.tested, 10);
        assert_eq!(r.dead, 7);
        assert!(!r.should_recover());
    }

    /// 路由组路径：全部生效节点死亡时触发（比例路径不满足的场景）。
    #[test]
    fn all_routing_groups_broken_triggers_recovery() {
        // 只有 1 个已测节点且已死；自动优选 / 扶梯出行 / 人工智能 的生效
        // 节点全部解析到它：比例路径不满足（tested=1 < 3），路由组路径必须触发。
        let proxies = fixture(&[("Only", 0)]);
        let r = analyze(&proxies);
        assert_eq!(r.tested, 1);
        assert_eq!(r.dead, 1);
        assert!(r.routing_total >= MIN_ROUTING_GROUPS);
        assert_eq!(r.routing_broken, r.routing_total);
        assert!(r.should_recover());
    }

    #[test]
    fn partially_broken_routing_does_not_trigger() {
        // 自动优选 → 存活节点 A；扶梯出行/人工智能 → 自动优选 → A：
        // 路由组全部健康（人工优选手动选中的 B 已死，但它不在路由判定集）；
        // 比例 2/3 也低于阈值。两条路径都不触发。
        let proxies = fixture(&[("A", 120), ("B", 0), ("C", 0)]);
        let r = analyze(&proxies);
        assert_eq!(r.routing_broken, 0);
        assert!(!r.should_recover());
    }

    #[test]
    fn malformed_input_is_safe() {
        // 空对象 / 缺 proxies 键：全零报告，不触发。
        assert!(!analyze(&serde_json::json!({})).should_recover());
        assert!(!analyze(&serde_json::json!({ "proxies": [] })).should_recover());
        assert!(!analyze(&serde_json::Value::Null).should_recover());
    }
}
