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
    crate::update::current_version().to_string()
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

/// mihomo 日志里每条连接记录都带目标域名 / IP（`[TCP] 127.0.0.1:5000 --> example.com:443 match …`），
/// 属于浏览历史。诊断包只保留非连接类的行（启动 / 监听 / 错误 / 规则加载等）。
fn scrub_connection_lines(text: &str) -> String {
    let mut dropped = 0usize;
    let mut out = String::new();
    for line in text.lines() {
        if line.contains(" --> ") || line.contains("-->") && line.contains("match ") {
            dropped += 1;
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    if dropped > 0 {
        out.push_str(&format!(
            "({} connection lines omitted for privacy)\n",
            dropped
        ));
    }
    out
}

/// 只保留最新的 `keep` 份诊断文件，避免数据目录里无限堆积。
fn prune_old_diagnostics(dir: &std::path::Path, keep: usize) {
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("diagnostics-") && n.ends_with(".txt"))
        })
        .collect();
    files.sort(); // 文件名带 unix 秒，字典序即时间序
    let excess = files.len().saturating_sub(keep);
    for f in files.into_iter().take(excess) {
        let _ = std::fs::remove_file(f);
    }
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
    let _ = writeln!(out, "app version : {}", crate::update::current_version());
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
        out.push_str(&scrub_connection_lines(&tail_lines(&logs.join(name), 200)));
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
    prune_old_diagnostics(&data_dir, 3);
    Ok(path.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrub_drops_connection_lines_but_keeps_errors() {
        let log = "time=1 level=info msg=\"[TCP] 127.0.0.1:5000 --> secret-site.example:443 match Match using DIRECT\"\ntime=2 level=error msg=\"Start TCP listening error\"\n";
        let out = scrub_connection_lines(log);
        assert!(!out.contains("secret-site.example"), "{}", out);
        assert!(out.contains("Start TCP listening error"));
        assert!(out.contains("1 connection lines omitted"));
    }

    #[test]
    fn prune_keeps_only_the_newest_diagnostics() {
        let d = std::env::temp_dir().join(format!("clashedge-diag-prune-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        for n in [100, 200, 300, 400, 500] {
            std::fs::write(d.join(format!("diagnostics-{}.txt", n)), "x").unwrap();
        }
        std::fs::write(d.join("config.yaml"), "keep").unwrap();
        prune_old_diagnostics(&d, 3);
        assert!(!d.join("diagnostics-100.txt").exists());
        assert!(!d.join("diagnostics-200.txt").exists());
        assert!(d.join("diagnostics-500.txt").exists());
        assert!(d.join("config.yaml").exists());
        let _ = std::fs::remove_dir_all(&d);
    }
}
