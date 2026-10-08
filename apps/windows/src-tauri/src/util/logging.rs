// src-tauri/src/util/logging.rs
//! 日志初始化，使用 tracing + tauri-plugin-log（v2 API）
//!
//! tauri-plugin-log 2.x：`Target` / `TargetKind`，级别用 `log::LevelFilter`。
//! 需在 Cargo.toml 启用 `tracing` feature，将 `tracing` 宏桥接到 log 层。
//!
//! 数据分离：便携模式下日志属于用户数据，写入 `<exe_dir>/Data/logs` 随包迁移，
//! 而不是 OS 日志目录 `%APPDATA%`（否则整体复制/换电脑后日志散落在用户目录）。

use log::LevelFilter;
use tauri_plugin_log::{RotationStrategy, Target, TargetKind};

/// 构建日志插件（供 main.rs 的 `.plugin(...)` 使用）
pub fn init() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    let mut targets = vec![
        Target::new(TargetKind::Stdout),
        Target::new(TargetKind::Webview),
    ];

    // 便携模式：日志写入 `<exe_dir>/Data/logs`（目录不存在则尝试创建）。
    // 创建失败或无法判定便携根目录时回退 OS 日志目录，避免静默丢日志。
    let mut pushed_data_logs = false;
    if crate::util::paths::is_portable_mode() {
        if let Some(root) = crate::util::paths::portable_root() {
            let log_dir = root.join("Data").join("logs");
            if std::fs::create_dir_all(&log_dir).is_ok() {
                targets.push(Target::new(TargetKind::Folder {
                    path: log_dir,
                    file_name: None,
                }));
                pushed_data_logs = true;
            }
        }
    }
    if !pushed_data_logs {
        targets.push(Target::new(TargetKind::LogDir { file_name: None }));
    }

    // 应用自身日志级别：debug 构建 Debug；release 默认 Info。
    // 排查问题时可设置环境变量 `CLASH_EDGE_LOG=debug`（或 trace）开启诊断级别。
    let app_level = match std::env::var("CLASH_EDGE_LOG")
        .map(|v| v.to_ascii_lowercase())
        .as_deref()
    {
        Ok("trace") => LevelFilter::Trace,
        Ok("debug") => LevelFilter::Debug,
        _ if cfg!(debug_assertions) => LevelFilter::Debug,
        _ => LevelFilter::Info,
    };

    tauri_plugin_log::Builder::new()
        .targets(targets)
        // 默认单文件上限很小，且只保留 1 份，崩溃现场很快被轮转掉；
        // 显式设置 2 MiB 并保留最近 5 份。
        .max_file_size(2 * 1024 * 1024)
        .rotation_strategy(RotationStrategy::KeepSome(5))
        .level_for("clash_edge", app_level)
        .level(LevelFilter::Info)
        .build()
}
