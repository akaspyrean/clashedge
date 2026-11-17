// src-tauri/src/core/logs.rs
//! 日志流：从 mihomo 外部控制器 `/logs`（SSE）拉取日志并转发给前端。
//!
//! `/logs` 是长连接（text/event-stream）：普通 GET 会一直挂起直到超时，
//! 不能像其它 REST 端点一样一次读完（这就是"日志需通过外部控制器接口获取，
//! 尚未接入"占位的原因）。这里由后端维持连接、逐行解析，把每条日志以
//! `log-line` 事件推给前端；控制器密钥只存在于后端，前端不接触。

use std::time::Duration;

use tauri::{AppHandle, Emitter};

/// SSE 单行缓冲上限（1 MiB）
const MAX_LINE_BUFFER: usize = 1024 * 1024;
/// 硬性总缓冲上限（4 MiB）：即使有换行也会强制清理，防止长期高频日志无限累积
const MAX_TOTAL_BUFFER: usize = 4 * 1024 * 1024;

/// 重连退避：断开后从 2s 起指数翻倍，封顶 30s；
/// 每次 HTTP 200 连接成功后重置回初始值。核心长时间不可达时
/// 不再以固定 2s 频率空转重试。
const RECONNECT_BACKOFF_INITIAL: Duration = Duration::from_secs(2);
const RECONNECT_BACKOFF_MAX: Duration = Duration::from_secs(30);

/// 事件名（后端 → 前端）
pub const EVENT_LOG_LINE: &str = "log-line";
pub const EVENT_LOG_CONNECTED: &str = "log-connected";
pub const EVENT_LOG_ERROR: &str = "log-error";

/// 批量推送：最多积攒这么多条日志立即发送，避免每行一次 IPC。
const LOG_BATCH_MAX_ENTRIES: usize = 32;
/// 批量推送：积攒日志后最久等待这么久发送，避免低频日志延迟过高。
const LOG_BATCH_MAX_WAIT: Duration = Duration::from_millis(50);

/// 启动日志流任务：连接 `{base}/logs`，逐行解析 SSE，emit 事件。
///
/// - 连接成功 → emit `log-connected`，随后每行 `log-line {level, message}`；
/// - 断开后按指数退避自动重连（2s → 翻倍 → 上限 30s；连接成功后重置），
///   核心重启/退出期间持续重试；
/// - 仅在"连通 → 断开"转换时上报一次 `log-error`，避免断线期间刷屏。
///
/// 返回的 JoinHandle 由命令层持有；前端离开日志页时调用 `stop_log_stream`
/// 对其 `abort()`，保证不残留后台连接。
pub fn spawn_log_stream(
    app: AppHandle,
    controller: &str,
    secret: &str,
) -> tauri::async_runtime::JoinHandle<()> {
    let controller = controller.to_string();
    let secret = secret.to_string();

    tauri::async_runtime::spawn(async move {
        let base = if controller.starts_with("http://") || controller.starts_with("https://") {
            controller
        } else {
            format!("http://{}", controller)
        };
        // level=debug：SSE 请求全量日志（mihomo 侧不再按 log-level 预筛），
        // 前端已有级别筛选芯片负责二次过滤。空闲期也有 debug 输出，页面不再长时间空白。
        let url = format!("{}/logs?level=debug", base);
        // 控制器在回环地址上，忽略环境/系统代理（审计 B6）。
        // 添加连接超时，避免核心不可达时长连接无限挂起。
        let client = reqwest::Client::builder()
            .no_proxy()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        let mut down = false;
        // 退避时长随连续失败翻倍，成功建立 HTTP 200 会话后重置。
        let mut backoff = RECONNECT_BACKOFF_INITIAL;
        // 与 CoreManager 同源构造 Authorization 头；非法密钥字符显式上报
        // （log-error 事件 + warn），不再静默省略导致 401 被误读为控制器不可达。
        let mut headers = reqwest::header::HeaderMap::new();
        match crate::core::controller::authorization_headers(&secret) {
            Ok(h) => headers = h,
            Err(e) => {
                tracing::warn!("Invalid controller secret for log stream: {}", e);
                emit_error(&app, &mut down, &e.to_string());
            }
        }

        loop {
            // 本轮 outcome：HTTP 200 建立了会话（不关心流多久）视为成功。
            let mut session_up = false;
            match client.get(&url).headers(headers.clone()).send().await {
                Ok(resp) if resp.status().is_success() => {
                    down = false;
                    session_up = true;
                    let _ = app.emit(EVENT_LOG_CONNECTED, ());
                    let mut stream = resp;
                    let mut buffer = Vec::new();
                    // 批量积攒的日志条目，满 32 条或 50ms 统一 emit。
                    let mut batch: Vec<serde_json::Value> =
                        Vec::with_capacity(LOG_BATCH_MAX_ENTRIES);
                    let mut last_emit = std::time::Instant::now();
                    loop {
                        match stream.chunk().await {
                            Ok(Some(bytes)) => {
                                buffer.extend_from_slice(&bytes);
                                // 防御异常长行：缓冲无换行时不无限增长
                                if buffer.len() > MAX_LINE_BUFFER && !buffer.contains(&b'\n') {
                                    buffer.clear();
                                }
                                // 硬性总缓冲上限：即使有换行也强制清理，防止长期高频日志无限累积
                                if buffer.len() > MAX_TOTAL_BUFFER {
                                    buffer.clear();
                                }
                                // 逐行处理（SSE 事件以 \n 分隔，空行/注释行跳过）
                                // 使用游标索引 + copy_within，避免 drain(..=pos) 的 O(N) 搬移。
                                let mut pos = 0usize;
                                while pos < buffer.len() {
                                    if let Some(newline_pos) =
                                        buffer[pos..].iter().position(|&b| b == b'\n')
                                    {
                                        let line_end = pos + newline_pos + 1;
                                        let line = &buffer[pos..line_end];
                                        if let Some(entry) = parse_line(line) {
                                            batch.push(entry);
                                            if batch.len() >= LOG_BATCH_MAX_ENTRIES {
                                                flush_batch(&app, &mut batch, &mut last_emit);
                                            }
                                        }
                                        pos = line_end;
                                    } else {
                                        break;
                                    }
                                }
                                // 压缩已处理的前缀，保留未处理的尾部
                                if pos > 0 {
                                    buffer.copy_within(pos.., 0);
                                    buffer.truncate(buffer.len() - pos);
                                }
                            }
                            Ok(None) => break, // 服务端关闭连接（核心重启/退出）
                            Err(e) => {
                                emit_error(&app, &mut down, &e.to_string());
                                break;
                            }
                        }
                        // 定时刷新批量日志，避免低频日志延迟过高
                        if !batch.is_empty() && last_emit.elapsed() >= LOG_BATCH_MAX_WAIT {
                            flush_batch(&app, &mut batch, &mut last_emit);
                        }
                    }
                    // 连接断开前 flush 剩余批次
                    if !batch.is_empty() {
                        flush_batch(&app, &mut batch, &mut last_emit);
                    }
                }
                Ok(resp) => {
                    emit_error(
                        &app,
                        &mut down,
                        &format!("Controller returned {}", resp.status()),
                    );
                }
                Err(e) => {
                    emit_error(&app, &mut down, &e.to_string());
                }
            }
            // 成功路径重置退避；失败路径翻倍封顶，
            // 避免核心不可达时以 2s 常频空转。
            if session_up {
                backoff = RECONNECT_BACKOFF_INITIAL;
            } else {
                backoff = backoff.saturating_mul(2);
                if backoff > RECONNECT_BACKOFF_MAX {
                    backoff = RECONNECT_BACKOFF_MAX;
                }
            }
            tokio::time::sleep(backoff).await;
        }
    })
}

/// 解析一行 SSE，返回可直接 emit 的 JSON 值，解析失败返回 None。
fn parse_line(line: &[u8]) -> Option<serde_json::Value> {
    let text = String::from_utf8_lossy(line).trim().to_string();
    if text.is_empty() || text.starts_with(':') {
        return None;
    }
    // SSE 行形如 `data: {json}`；也容忍裸 JSON 行。
    let json_str = text.strip_prefix("data:").map(str::trim).unwrap_or(&text);
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str) else {
        return None;
    };
    let level = v
        .get("type")
        .and_then(|t| t.as_str())
        .unwrap_or("info")
        .to_string();
    let message = v
        .get("payload")
        .and_then(|p| p.as_str())
        .unwrap_or("")
        .to_string();
    Some(serde_json::json!({ "level": level, "message": message }))
}

/// 将批量积攒的日志条目以单个 IPC 事件发送给前端。
fn flush_batch(
    app: &AppHandle,
    batch: &mut Vec<serde_json::Value>,
    last_emit: &mut std::time::Instant,
) {
    if batch.is_empty() {
        return;
    }
    let entries = std::mem::take(batch);
    let _ = app.emit(EVENT_LOG_LINE, serde_json::json!({ "batch": entries }));
    *last_emit = std::time::Instant::now();
}

/// 仅在"由连通转为断开"时上报一次，避免断线后每 2 秒刷屏。
fn emit_error(app: &AppHandle, down: &mut bool, error: &str) {
    if !*down {
        *down = true;
        let _ = app.emit(EVENT_LOG_ERROR, serde_json::json!({ "error": error }));
    }
}
