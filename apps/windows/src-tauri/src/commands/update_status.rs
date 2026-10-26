// src-tauri/src/commands/update_status.rs
//! 自动更新状态查询 + 「节点池整体不可用」自愈触发。
//!
//! 状态查询（供首页展示"何时更新、成功还是失败"）返回结构：
//! ```json
//! {
//!   "geo": { "attempt": 0, "success": 0, "ok": false, "error": "" },
//!   "subscriptions": { "MySub": { ... } },
//!   "geo_enabled": true,
//!   "subscription_enabled": true
//! }
//! ```
//! - `attempt`：最后一次尝试时间（unix 秒，0 = 从未尝试）
//! - `success`：最后一次成功时间（unix 秒，0 = 从未成功）
//! - `ok`：最后一次尝试是否成功
//! - `error`：失败原因（成功时为空串）
//!
//! 自愈触发：前端测速发现**所有节点都超时或全部异常慢**时调用
//! `trigger_auto_recovery`，由后端立即刷新**节点数据（订阅）**与
//! **地理数据/规则**（节点池整体异常通常是订阅过期或 GeoData/规则陈旧所致）。
//! **限流**：为避免反复打爆上游，两次触发至少间隔
//! [`AUTO_RECOVERY_COOLDOWN_SECS`]；两个自动更新开关分别生效。

use crate::util::error::Result;
use crate::util::update_status;
use tauri::{command, Manager, State};

/// 两次「节点池整体不可用」触发之间的最小间隔（秒）。
///
/// 取 10 分钟：足够覆盖一次"订阅已死 → 用户反复点测速"的抖动，
/// 又不会让真正的自愈来得太慢（用户仍可手动点「更新订阅」）。
const AUTO_RECOVERY_COOLDOWN_SECS: u64 = 10 * 60;

/// 限流状态在状态文件里的键（保留前缀，避免与 `subscription:` 命名空间冲突）。
const KEY_LAST_RECOVERY_TRIGGER: &str = "__last_timeout_trigger";

#[command]
pub fn get_auto_update_status(
    app: tauri::AppHandle,
    state: State<'_, crate::AppState>,
) -> Result<serde_json::Value> {
    let data_dir = crate::util::paths::get_app_data_dir(&app)?;
    let all = update_status::read_all(&data_dir);

    let geo = update_status::read_one(&data_dir, update_status::KEY_GEO).unwrap_or_default();
    let mut subscriptions = serde_json::Map::new();
    for (key, rec) in &all {
        if let Some(name) = key.strip_prefix(update_status::KEY_SUBSCRIPTION_PREFIX) {
            subscriptions.insert(
                name.to_string(),
                serde_json::to_value(rec).unwrap_or_default(),
            );
        }
    }

    // 自动更新开关状态（用户可关；关掉时前端应显示"已关闭"而不是"失败"）
    let (geo_enabled, subscription_enabled) = {
        let cfg = state.config_manager.lock().unwrap().get_config();
        (
            cfg.general.geo_auto_update,
            cfg.general.auto_update_subscription,
        )
    };

    Ok(serde_json::json!({
        "geo": serde_json::to_value(geo).unwrap_or_default(),
        "subscriptions": serde_json::Value::Object(subscriptions),
        "geo_enabled": geo_enabled,
        "subscription_enabled": subscription_enabled,
    }))
}

/// 节点池整体不可用（全部超时 / 全部异常慢）→ 立即刷新节点数据 + 地理数据/规则。
///
/// 与后台定时任务的区别：不等待 6h / 24h 的"到期"窗口——用户此刻已经无法
/// 正常选路，等定时器没有意义；但仍遵守各自的自动更新开关与 10 分钟冷却。
///
/// 幂等 + 限流：
/// - 距上次触发不足 [`AUTO_RECOVERY_COOLDOWN_SECS`] → 直接跳过（返回 `cooldown`）；
/// - 两个开关都关闭 → 尊重用户意图，不触发；
/// - 任一开关开启 → 只执行对应更新；单项失败不影响另一项。
///
/// 返回 `{ "triggered": bool, "refreshed": usize, "geo_updated": bool, "reason": "..." }`。
#[command]
pub async fn trigger_auto_recovery(app: tauri::AppHandle) -> Result<serde_json::Value> {
    let data_dir = crate::util::paths::get_app_data_dir(&app)?;

    // 尊重用户的开关设置：关掉的项目不自动拉取。
    let (subscription_enabled, geo_enabled) = {
        let state = app.state::<crate::AppState>();
        let guard = state.config_manager.lock().unwrap();
        let cfg = guard.get_config();
        (
            cfg.general.auto_update_subscription,
            cfg.general.geo_auto_update,
        )
    };
    if !subscription_enabled && !geo_enabled {
        return Ok(serde_json::json!({
            "triggered": false,
            "refreshed": 0,
            "geo_updated": false,
            "reason": "auto-update disabled by user",
        }));
    }

    // 限流：距上次触发不足冷却窗口则跳过。
    let now = update_status::now_secs();
    let last = update_status::read_one(&data_dir, KEY_LAST_RECOVERY_TRIGGER)
        .map(|r| r.attempt)
        .unwrap_or(0);
    if last > 0 && now.saturating_sub(last) < AUTO_RECOVERY_COOLDOWN_SECS {
        return Ok(serde_json::json!({
            "triggered": false,
            "refreshed": 0,
            "geo_updated": false,
            "reason": "cooldown",
        }));
    }

    // 先落限流标记（即使后续更新失败，也不该在短时间内反复打上游）。
    update_status::record(&data_dir, KEY_LAST_RECOVERY_TRIGGER, true, "");

    // 1) 节点数据：串行刷新全部订阅（对上游友好）。
    let refreshed = if subscription_enabled {
        crate::commands::profiles::refresh_all_subscriptions(&app).await
    } else {
        0
    };

    // 2) 地理数据/规则：立即执行（跳过 24h 到期判断，受开关约束）。
    let geo_updated = if geo_enabled {
        match crate::geodata::updater::update_geodata(&app).await {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!("Auto-recovery geo/rules update failed: {}", e);
                false
            }
        }
    } else {
        false
    };

    tracing::info!(
        "Auto-recovery after degraded node pool: {} profile(s) refreshed, geo/rules updated={}",
        refreshed,
        geo_updated
    );
    Ok(serde_json::json!({
        "triggered": true,
        "refreshed": refreshed,
        "geo_updated": geo_updated,
        "reason": "all nodes unavailable (timeout or abnormally slow)",
    }))
}
