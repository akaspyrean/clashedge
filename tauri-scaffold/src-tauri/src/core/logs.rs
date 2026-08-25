// src-tauri/src/core/logs.rs
//! 日志流：从 mihomo 外部控制器 `/logs`（SSE）拉取日志并转发给前端。
//!
//! `/logs` 是长连接（text/event-stream）：普通 GET 会一直挂起直到超时，
//! 不能像其它 REST 端点一样一次读完（这就是"日志需通过外部控制器接口获取，
//! 尚未接入"占位的原因）。这里由后端维持连接、逐行解析，把每条日志以
//! `log-line` 事件推给前端；控制器密钥只存在于后端，前端不接触。

use std::time::Duration;

use tauri::{AppHandle, Emitter};

/// 事件名（后端 → 前端）
pub const EVENT_LOG_LINE: &str = "log-line";
pub const EVENT_LOG_CONNECTED: &str = "log-connected";
pub const EVENT_LOG_ERROR: &str = "log-error";

/// 启动日志流任务：连接 `{base}/logs`，逐行解析 SSE，emit 事件。
///
/// - 连接成功 → emit `log-connected`，随后每行 `log-line {level, message}`；
/// - 断开后以 2s 间隔自动重连（核心重启/退出期间持续重试）；
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
        let url = format!("{}/logs", base);
        let client = reqwest::Client::new();
        let mut down = false;
        // 与 CoreManager 同源构造 Authorization 头；非法密钥字符显式上报
        // （log-error 事件 + warn），不再静默省略导致 401 被误读为控制器不可达。
        let mut headers = reqwest::header::HeaderMap::new();
        match crate::core::manager::authorization_headers(&secret) {
            Ok(h) => headers = h,
            Err(e) => {
                tracing::warn!("Invalid controller secret for log stream: {}", e);
                emit_error(&app, &mut down, &e.to_string());
            }
        }

        loop {
            match client.get(&url).headers(headers.clone()).send().await {
                Ok(resp) if resp.status().is_success() => {
                    down = false;
                    let _ = app.emit(EVENT_LOG_CONNECTED, ());
                    let mut stream = resp;
                    let mut buffer = Vec::new();
                    loop {
                        match stream.chunk().await {
                            Ok(Some(bytes)) => {
                                buffer.extend_from_slice(&bytes);
                                // 逐行处理（SSE 事件以 \n 分隔，空行/注释行跳过）
                                while let Some(pos) = buffer.iter().position(|&b| b == b'\n') {
                                    let line: Vec<u8> = buffer.drain(..=pos).collect();
                                    handle_line(&app, &line);
                                }
                            }
                            Ok(None) => break, // 服务端关闭连接（核心重启/退出）
                            Err(e) => {
                                emit_error(&app, &mut down, &e.to_string());
                                break;
                            }
                        }
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
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    })
}

/// 解析一行 SSE 并 emit `log-line`。
fn handle_line(app: &AppHandle, line: &[u8]) {
    let text = String::from_utf8_lossy(line).trim().to_string();
    if text.is_empty() || text.starts_with(':') {
        return;
    }
    // SSE 行形如 `data: {json}`；也容忍裸 JSON 行。
    let json_str = text.strip_prefix("data:").map(str::trim).unwrap_or(&text);
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str) else {
        return;
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
    let _ = app.emit(
        EVENT_LOG_LINE,
        serde_json::json!({ "level": level, "message": message }),
    );
}

/// 仅在"由连通转为断开"时上报一次，避免断线后每 2 秒刷屏。
fn emit_error(app: &AppHandle, down: &mut bool, error: &str) {
    if !*down {
        *down = true;
        let _ = app.emit(EVENT_LOG_ERROR, serde_json::json!({ "error": error }));
    }
}
