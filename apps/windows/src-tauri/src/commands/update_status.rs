// src-tauri/src/commands/update_status.rs
//! 自动更新状态查询 + 「节点全超时」自愈触发。
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
//! 自愈触发：前端测速发现**所有节点都超时**时调用 `trigger_subscription_refresh`，
//! 由后端立即拉取全部订阅（节点全挂通常是订阅过期所致）。
//! **限流**：为避免反复打爆订阅服务器，两次触发至少间隔
//! [`TIMEOUT_REFRESH_COOLDOWN_SECS`]。

use crate::util::error::Result;
use crate::util::update_status;
use tauri::{command, Manager, State};

/// 两次「节点全超时」触发之间的最小间隔（秒）。
///
/// 取 10 分钟：足够覆盖一次"订阅已死 → 用户反复点测速"的抖动，
/// 又不会让真正的自愈来得太慢（用户仍可手动点「更新订阅」）。
const TIMEOUT_REFRESH_COOLDOWN_SECS: u64 = 10 * 60;

/// 限流状态在状态文件里的键（保留前缀，避免与 `subscription:` 命名空间冲突）。
const KEY_LAST_TIMEOUT_TRIGGER: &str = "__last_timeout_trigger";

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

/// 节点全部超时 → 立即拉取全部订阅（自愈）。
///
/// 幂等 + 限流：
/// - 距上次触发不足 [`TIMEOUT_REFRESH_COOLDOWN_SECS`] → 直接跳过（返回 `skipped`）；
/// - 正在刷新中 → 直接跳过（返回 `skipped`），避免并发重复拉取；
/// - 用户关闭了「自动更新订阅」→ 尊重用户意图，不触发。
///
/// 返回 `{ "triggered": bool, "refreshed": usize, "reason": "..." }`。
#[command]
pub async fn trigger_subscription_refresh(app: tauri::AppHandle) -> Result<serde_json::Value> {
    let data_dir = crate::util::paths::get_app_data_dir(&app)?;

    // 尊重用户的开关设置：关掉了自动更新订阅就不自动拉取。
    let enabled = {
        let state = app.state::<crate::AppState>();
        let guard = state.config_manager.lock().unwrap();
        let cfg = guard.get_config();
        cfg.general.auto_update_subscription
    };
    if !enabled {
        return Ok(serde_json::json!({
            "triggered": false,
            "refreshed": 0,
            "reason": "auto-update disabled by user",
        }));
    }

    // 限流：距上次触发不足冷却窗口则跳过。
    let now = update_status::now_secs();
    let last = update_status::read_one(&data_dir, KEY_LAST_TIMEOUT_TRIGGER)
        .map(|r| r.attempt)
        .unwrap_or(0);
    if last > 0 && now.saturating_sub(last) < TIMEOUT_REFRESH_COOLDOWN_SECS {
        return Ok(serde_json::json!({
            "triggered": false,
            "refreshed": 0,
            "reason": "cooldown",
        }));
    }

    // 先落限流标记（即使后续刷新失败，也不该在短时间内反复打上游）。
    update_status::record(&data_dir, KEY_LAST_TIMEOUT_TRIGGER, true, "");

    // 立即刷新全部订阅（串行，对上游友好）。
    let refreshed = crate::commands::profiles::refresh_all_subscriptions(&app).await;

    tracing::info!(
        "Triggered subscription refresh after all nodes timed out: {} profile(s) refreshed",
        refreshed
    );
    Ok(serde_json::json!({
        "triggered": true,
        "refreshed": refreshed,
        "reason": "all nodes timed out",
    }))
}
