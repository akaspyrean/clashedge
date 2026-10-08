// src-tauri/src/commands/util.rs
//! 工具命令：打开目录、版本、语言等

use crate::util::error::Result;
use tauri::{command, Emitter, Manager};

#[command]
pub async fn open_data_dir(app: tauri::AppHandle) -> Result<()> {
    let data_dir = crate::util::paths::get_app_data_dir(&app)?;
    crate::util::paths::open_in_explorer(&data_dir)
}

#[command]
pub async fn open_logs_dir(app: tauri::AppHandle) -> Result<()> {
    let logs_dir = crate::util::paths::get_logs_dir(&app)?;
    crate::util::paths::open_in_explorer(&logs_dir)
}

#[command]
pub fn get_app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// 是否由自启动拉起（命令行带 --clash-edge-autostart）。
/// 自启动时应只驻留托盘不显示窗口，手动启动才显示主窗口。
#[command]
pub fn is_autostart() -> bool {
    std::env::args().any(|a| a == "--clash-edge-autostart")
}

/// 当前是否已开启开机自启（注册表 Run 键 + StartupApproved）。
#[command]
pub fn get_autostart() -> Result<bool> {
    crate::util::autostart::get_autostart()
}

/// 设置开机自启。启用时注册表 Run 键指向根启动器 `--clash-edge-autostart`，
/// 保证开机静默启动。完成后刷新托盘菜单（开机自启项勾选态跟随）。
#[command]
pub async fn set_autostart(app: tauri::AppHandle, enable: bool) -> Result<()> {
    crate::util::autostart::set_autostart(enable)?;
    crate::core::runtime::refresh_tray(&app).await?;
    let _ = app.emit("autostart-changed", serde_json::json!({ "enable": enable }));
    Ok(())
}

#[command]
pub fn get_supported_locales() -> Vec<String> {
    crate::i18n::loader::supported_locales()
        .iter()
        .map(|s| s.to_string())
        .collect()
}

#[command]
pub async fn set_locale(app: tauri::AppHandle, locale: String) -> Result<()> {
    // 经 AppController：事务锁 + 降级守卫 + 持久化 + 托盘刷新
    let state = app.state::<crate::AppState>();
    state.controller.set_locale(&app, locale).await
}

#[command]
pub fn get_i18n_messages(locale: String) -> Result<serde_json::Value> {
    let table = crate::i18n::loader::messages_for_locale(&locale);
    Ok(serde_json::to_value(table)?)
}

/// 取文本文件末尾约 `max_lines` 行（只读尾部 256 KiB，避免把大日志整个读进内存）。
fn tail_lines(path: &std::path::Path, max_lines: usize) -> String {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(path) else {
        return String::from("(missing)\n");
    };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let start = len.saturating_sub(256 * 1024);
    if f.seek(SeekFrom::Start(start)).is_err() {
        return String::from("(unreadable)\n");
    }
    let mut buf = Vec::new();
    let _ = f.read_to_end(&mut buf);
    let text = String::from_utf8_lossy(&buf);
    let lines: Vec<&str> = text.lines().collect();
    let from = lines.len().saturating_sub(max_lines);
    let mut out = lines[from..].join("\n");
    out.push('\n');
    out
}

/// 导出诊断包（文本）：版本 / 运行状态 / **脱敏**后的配置 / 最近日志。
/// 不包含控制器密钥、节点（订阅内容）与订阅地址。返回生成文件的路径。
#[command]
pub async fn export_diagnostics(app: tauri::AppHandle) -> Result<String> {
    use std::fmt::Write as _;
    let state = app.state::<crate::AppState>();
    let data_dir = crate::util::paths::get_app_data_dir(&app)?;

    let mut out = String::new();
    let _ = writeln!(out, "ClashEdge diagnostics");
    let _ = writeln!(out, "app version : {}", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(
        out,
        "os / arch   : {} / {}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    let _ = writeln!(
        out,
        "elevated    : {}",
        crate::util::elevation::is_elevated()
    );
    let _ = writeln!(
        out,
        "layout      : {}",
        crate::util::paths::portable_mode_diagnostic()
    );
    if let Some(core) = state.core_manager.get() {
        let status = core.get_status().await;
        let _ = writeln!(out, "core        : {}", status);
    }

    // 脱敏配置：去掉 secret 与 extra（导入配置可能带节点）
    let mut cfg = serde_json::to_value(state.config_manager.lock().unwrap().get_config())?;
    if let Some(obj) = cfg.as_object_mut() {
        obj.remove("secret");
        obj.remove("extra");
    }
    let _ = writeln!(out, "\n===== config (redacted) =====");
    let _ = writeln!(out, "{}", serde_json::to_string_pretty(&cfg)?);

    let logs = data_dir.join("logs");
    for name in ["mihomo-stderr.log", "mihomo-stdout.log"] {
        let _ = writeln!(out, "\n===== {} (tail) =====", name);
        out.push_str(&tail_lines(&logs.join(name), 200));
    }
    // 应用日志：取 logs 目录里最新修改的 *.log（排除 mihomo 日志）
    let newest_app_log = std::fs::read_dir(&logs)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            n.ends_with(".log") && !n.starts_with("mihomo-")
        })
        .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok());
    if let Some(e) = newest_app_log {
        let _ = writeln!(
            out,
            "\n===== {} (tail) =====",
            e.file_name().to_string_lossy()
        );
        out.push_str(&tail_lines(&e.path(), 300));
    }

    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = data_dir.join(format!("diagnostics-{}.txt", stamp));
    crate::util::atomic::atomic_write(&path, out.as_bytes())?;
    Ok(path.to_string_lossy().to_string())
}
