// src-tauri/src/util/update_status.rs
//! 自动更新结果记录（供首页展示"何时更新、成功还是失败"）。
//!
//! 记录内容写到 `Data/auto-update-status.json`：
//! - `attempt`：最后一次**尝试**的 unix 秒（无论成败）
//! - `success`：最后一次**成功**的 unix 秒（从未成功为 0）
//! - `ok`：最后一次尝试是否成功
//! - `error`：失败原因（成功时为空串）
//!
//! 键名约定：
//! - `geo`：GeoIP/GeoSite + 规则集更新
//! - `subscription:<profile 名>`：单个订阅的自动刷新
//!
//! 设计约束：
//! - **写失败不影响主流程**：自动更新失败本身不能因为"记录状态失败"而变成
//!   更严重的错误；写盘失败只记 warn。
//! - **不记录敏感信息**：错误文本在写入前做长度截断，避免把订阅 URL/token
//!   写进状态文件。
//! - **原子写**：与其它配置一致，走 `util::atomic::atomic_write`。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::util::error::Result;

/// 状态文件名（位于应用数据目录）。
const STATUS_FILE: &str = "auto-update-status.json";

/// 错误文本最大保留长度（避免把超长响应体/带 token 的 URL 写进状态文件）。
const MAX_ERROR_LEN: usize = 200;

/// 单项更新记录。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdateRecord {
    /// 最后一次尝试（unix 秒）；0 = 从未尝试
    #[serde(default)]
    pub attempt: u64,
    /// 最后一次成功（unix 秒）；0 = 从未成功
    #[serde(default)]
    pub success: u64,
    /// 最后一次尝试是否成功
    #[serde(default)]
    pub ok: bool,
    /// 失败原因（成功时为空串）
    #[serde(default)]
    pub error: String,
    /// 连续失败次数；成功后归零，用于计算退避。
    #[serde(default)]
    pub failure_count: u32,
    /// 最早允许下次自动重试的 unix 秒；0 表示无退避限制。
    #[serde(default)]
    pub retry_after: u64,
}

impl UpdateRecord {
    /// 记录一次成功。
    pub fn success_at(now: u64) -> Self {
        Self {
            attempt: now,
            success: now,
            ok: true,
            error: String::new(),
            failure_count: 0,
            retry_after: 0,
        }
    }

    /// 记录一次失败（保留历史成功时间，便于展示"上次成功是何时"）。
    pub fn failure_at(now: u64, prev_success: u64, failure_count: u32, error: &str) -> Self {
        Self {
            attempt: now,
            success: prev_success,
            ok: false,
            error: sanitize_error(error),
            failure_count,
            retry_after: now.saturating_add(retry_delay_secs(failure_count)),
        }
    }
}

/// 截断并清洗错误文本（去掉控制字符、限长）。
fn sanitize_error(msg: &str) -> String {
    let cleaned: String = msg
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.chars().count() > MAX_ERROR_LEN {
        let mut s: String = trimmed.chars().take(MAX_ERROR_LEN).collect();
        s.push('…');
        s
    } else {
        trimmed.to_string()
    }
}

/// Automatic retry backoff: 5m, 10m, 20m ... capped at 6h.
fn retry_delay_secs(failures: u32) -> u64 {
    (5 * 60u64)
        .saturating_mul(1u64 << failures.saturating_sub(1).min(7))
        .min(6 * 60 * 60)
}

/// Whether an automatic retry is due for the given update key.
pub fn retry_due(data_dir: &Path, key: &str, now: u64) -> bool {
    read_one(data_dir, key)
        .map(|record| record.retry_after == 0 || now >= record.retry_after)
        .unwrap_or(true)
}

/// 当前 unix 秒。
pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 状态文件路径。
fn status_path(data_dir: &Path) -> PathBuf {
    data_dir.join(STATUS_FILE)
}

/// 读取全部记录（文件缺失/损坏 → 空表）。
pub fn read_all(data_dir: &Path) -> HashMap<String, UpdateRecord> {
    let path = status_path(data_dir);
    let Ok(content) = std::fs::read_to_string(&path) else {
        return HashMap::new();
    };
    serde_json::from_str(&content).unwrap_or_default()
}

/// 读取单项记录。
pub fn read_one(data_dir: &Path, key: &str) -> Option<UpdateRecord> {
    read_all(data_dir).remove(key)
}

/// 写入一项记录（原子写）。
///
/// 写失败只记 warn，不向上抛——自动更新的成败判定不应被"状态记录失败"污染。
///
/// 参数用 `ok` + `error` 文本而非 `Result`，避免要求调用方克隆错误类型。
pub fn record(data_dir: &Path, key: &str, ok: bool, error: &str) {
    let now = now_secs();
    let mut all = read_all(data_dir);
    let entry = if ok {
        UpdateRecord::success_at(now)
    } else {
        let previous = all.get(key);
        let prev_success = previous.map(|r| r.success).unwrap_or(0);
        let failures = previous
            .map(|r| r.failure_count.saturating_add(1))
            .unwrap_or(1);
        UpdateRecord::failure_at(now, prev_success, failures, error)
    };
    all.insert(key.to_string(), entry);
    if let Err(e) = write_all(data_dir, &all) {
        tracing::warn!("Failed to persist auto-update status: {}", e);
    }
}

/// 把整表写入磁盘（原子写）。
fn write_all(data_dir: &Path, all: &HashMap<String, UpdateRecord>) -> Result<()> {
    let body = serde_json::to_string_pretty(all)
        .map_err(|e| crate::util::error::Error::Other(e.to_string()))?;
    crate::util::atomic::atomic_write(&status_path(data_dir), body.as_bytes())
}

/// GeoData / 规则集自动更新的记录键。
pub const KEY_GEO: &str = "geo";

/// 订阅自动刷新的记录键前缀。
pub const KEY_SUBSCRIPTION_PREFIX: &str = "subscription:";

/// 构造订阅记录键。
pub fn subscription_key(profile: &str) -> String {
    format!("{}{}", KEY_SUBSCRIPTION_PREFIX, profile)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "clash-edge-update-status-{}-{}",
            std::process::id(),
            tag
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn records_success_and_failure() {
        let dir = temp_dir("rec");
        record(&dir, KEY_GEO, true, "");
        let r = read_one(&dir, KEY_GEO).expect("record exists");
        assert!(r.ok);
        assert!(r.success > 0);
        assert_eq!(r.error, "");

        record(&dir, KEY_GEO, false, "network down");
        let r = read_one(&dir, KEY_GEO).expect("record exists");
        assert!(!r.ok);
        assert!(r.attempt > 0);
        // 失败不应抹掉历史成功时间
        assert!(r.success > 0);
        assert_eq!(r.error, "network down");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn error_text_is_sanitized_and_truncated() {
        let dir = temp_dir("err");
        let long = "x".repeat(500);
        record(&dir, KEY_GEO, false, &long);
        let r = read_one(&dir, KEY_GEO).unwrap();
        assert!(r.error.chars().count() <= MAX_ERROR_LEN + 1);
        assert!(!r.error.contains('\n'));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn subscription_keys_are_namespaced() {
        let k = subscription_key("MySub");
        assert!(k.starts_with(KEY_SUBSCRIPTION_PREFIX));
        assert_ne!(k, KEY_GEO);
    }

    #[test]
    fn retry_is_deferred_until_persisted_backoff_expires() {
        let dir = temp_dir("retry-backoff");
        let key = subscription_key("retry-me");
        record(&dir, &key, false, "offline");
        let failed = read_one(&dir, &key).unwrap();
        assert!(!retry_due(&dir, &key, failed.attempt));
        assert!(retry_due(&dir, &key, failed.retry_after));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn failure_backoff_increases_and_success_clears_it() {
        let first = UpdateRecord::failure_at(1_000, 0, 1, "offline");
        let second = UpdateRecord::failure_at(1_000, 0, 2, "offline");
        assert_eq!(first.retry_after - first.attempt, 5 * 60);
        assert_eq!(second.retry_after - second.attempt, 10 * 60);
        let success = UpdateRecord::success_at(2_000);
        assert_eq!(success.failure_count, 0);
        assert_eq!(success.retry_after, 0);
    }

    #[test]
    fn missing_file_reads_as_empty() {
        let dir = temp_dir("missing");
        assert!(read_all(&dir).is_empty());
        assert!(read_one(&dir, KEY_GEO).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
