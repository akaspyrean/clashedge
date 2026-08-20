// src-tauri/src/util/autostart.rs
//! Windows 注册表自启动管理
//!
//! 自启动值固定为 `"<launcher>\ClashEdge.exe" --clash-edge-autostart`：
//! - 参考包 C# 启动器布局：launcher = 包根 `ClashEdge.exe`（负责设置
//!   `CLASH_EDGE_DATA_DIR`/`HOME`/`APPDATA` 并拉起内层应用）；
//! - 原生便携布局：launcher = 应用本体自身。
//! `repair_autostart` 会在便携包移动/改名后把 Run 键路径自愈重写为当前位置。
//!
//! 写注册表 `HKCU\...\Run` 时，值固定为 `"<root>\ClashEdge.exe" --clash-edge-autostart`。
//! 同时还维护 `HKCU\...\Explorer\StartupApproved\Run`（12 字节 REG_BINARY，
//! 首字节 0x02=启用 / 0x03=禁用）——Windows 启动管理器会用 StartupApproved
//! 覆盖 Run 键的启用状态，只写 Run 键不写 StartupApproved，用户在任务管理器
//! 里禁用后再由本程序启用会失效。

use crate::util::error::{Error, Result};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use winreg::enums::*;
use winreg::RegKey;

/// Run 键路径：开机自启列表
const RUN_KEY_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
/// StartupApproved 键路径：任务管理器「启用」开关实际存储处
const APPROVED_KEY_PATH: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
/// 自启注册表值名
const VALUE_NAME: &str = "ClashEdge";
/// 自启命令行参数（根启动器转发给内层 exe，内层据此静默驻托盘）
const AUTOSTART_ARG: &str = "--clash-edge-autostart";

/// 解析注册表 Run 键的自启值，提取可执行文件路径。
///
/// 格式形如 `"<path>\ClashEdge.exe" --clash-edge-autostart`，
/// 需去除外层引号并取第一个空格分隔的 token（可执行文件路径）。
/// 裸路径含空格时只取第一个 token（Windows Run 键对含空格路径必须加引号，
/// 因此未加引号的裸路径视为异常数据，能做多好做多好）。
/// 返回 `None` 表示值不是合法的引导路径（空、空引号、引号未闭合）。
pub fn parse_launcher_path(value: &std::ffi::OsStr) -> Option<PathBuf> {
    let s = value.to_string_lossy();
    let s = s.trim();

    // 引号包裹路径："C:\...\ClashEdge.exe" --arg
    if s.starts_with('"') {
        // 引号未闭合 → 非法，不猜测
        let inner_end = s[1..].find('"')?;
        let inner = &s[1..1 + inner_end];
        if inner.is_empty() {
            return None; // 空引号 "" → 非法
        }
        return Some(PathBuf::from(inner));
    }

    // 裸路径：取第一个空格分隔的 token
    let token = s.split_whitespace().next()?;
    if token.is_empty() {
        return None;
    }
    Some(PathBuf::from(token))
}

/// 判断两个路径是否指向同一文件（大小写不敏感比较，Windows 下 canonicalize 可能
/// 失败，因此先比较字符串上，若相同再尝试 canonicalize）。
fn paths_equal(a: &Path, b: &Path) -> bool {
    if a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy()) {
        return true;
    }
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => false,
    }
}

/// 便携模式下自启动路径的自愈。
///
/// 当便携包复制/移动/改名后，注册表 Run 键中存储的路径可能指向旧位置。
/// 本函数读取 Run 键，若其中存储的可执行文件路径与当前 `current_exe()`
/// 不一致（大小写不敏感），则重写为当前 exe 路径，保证开机自启指向正确。
///
/// 非便携模式下本函数不执行任何操作（返回 Ok(())）。
pub fn repair_autostart() -> Result<()> {
    use crate::util::paths;
    if !paths::is_portable_mode() {
        return Ok(());
    }

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let run = match hkcu.open_subkey_with_flags(RUN_KEY_PATH, KEY_READ | KEY_WRITE) {
        Ok(k) => k,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(Error::Io(e)),
    };

    let value: OsString = match run.get_value(VALUE_NAME) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(Error::Io(e)),
    };

    let stored = match parse_launcher_path(&value) {
        Some(p) => p,
        None => return Ok(()), // 格式不符，交给 set_autostart 维护
    };

    // 自启动值指向根启动器（参考包布局下可能是包根 ClashEdge.exe，不是内层
    // current_exe）。以此为准做路径自愈。
    let launcher = root_launcher_path();

    // 自启动值可能同时存在其他合法格式（如无引号路径）；仅当解析出的路径与
    // 根启动器不一致时才重写。`paths_equal` 做大小写不敏感 + canonicalize 比对。
    if paths_equal(&stored, &launcher) {
        return Ok(()); // 已指向根启动器，无需修复
    }

    tracing::info!(
        "Repairing autostart path: stored {:?} -> launcher {:?}",
        stored, launcher
    );
    let new_value = format!("\"{}\" {}", launcher.display(), AUTOSTART_ARG);
    run.set_value(VALUE_NAME, &new_value)?;

    // 注意：**不**触碰 StartupApproved。任务管理器里用户对「ClashEdge」的启用/禁用
    // 状态按值名存储在 StartupApproved\Run（REG_BINARY，不含启动路径），与 Run 值
    // 的路径内容无关；只重写路径不会改变该开关，用户手动关闭的自启不会被重新打开。

    Ok(())
}

/// 获取根启动器路径（开机自启 Run 键应指向它，而不是内层应用 exe）。
///
/// - 参考包 C# 启动器布局（`CLASH_EDGE_DATA_DIR` 已设置，包根 `Data/` 的父目录
///   存在 `ClashEdge.exe`）：返回包根 `<root>\ClashEdge.exe`。开机自启时必须走
///   启动器，否则 `CLASH_EDGE_DATA_DIR`/`HOME`/`APPDATA` 不会被设置，内层 exe
///   会误判数据目录。
/// - 原生便携布局（`App/portable.dat`，exe 自身就是应用本体）：返回 exe 自身。
/// - 非便携模式：返回 exe 自身。
fn root_launcher_path() -> PathBuf {
    use crate::util::paths;
    if let Some(root) = paths::portable_root() {
        let launcher = root.join("ClashEdge.exe");
        if launcher.exists() {
            return launcher;
        }
    }
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("ClashEdge.exe"))
}

/// 读取启动批准状态。
/// 返回 `None` 表示没有 StartupApproved 记录（视为未受覆盖）。
/// 返回 `Some(true)` = 已批准（首字节 0x02）；`Some(false)` = 被用户禁用（0x03）。
fn approved_state() -> Option<bool> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let approved = hkcu
        .open_subkey_with_flags(APPROVED_KEY_PATH, KEY_READ)
        .ok()?;
    let raw = approved.get_raw_value(VALUE_NAME).ok()?;
    let first = raw.bytes.first().copied()?;
    Some(first == 0x02)
}

/// 是否已开启开机自启。
///
/// 判定依据（三者同时成立才为 true）：
/// 1. Run 键下存在 `ClashEdge` 值；
/// 2. 该值包含自启参数 `--clash-edge-autostart`（指向根启动器，而不是裸内层 exe）；
/// 3. 未被 StartupApproved 标记为禁用。
pub fn get_autostart() -> Result<bool> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let run = hkcu.open_subkey_with_flags(RUN_KEY_PATH, KEY_READ)?;
    let value: OsString = run.get_value(VALUE_NAME)?;
    if !value.to_string_lossy().contains(AUTOSTART_ARG) {
        return Ok(false);
    }
    // 无 StartupApproved 记录 = 未受覆盖；有记录则取其批准状态
    if let Some(approved) = approved_state() {
        return Ok(approved);
    }
    Ok(true)
}

/// 写入 StartupApproved 状态（12 字节 REG_BINARY，首字节 0x02 启用 / 0x03 禁用）。
fn set_approved_state(enable: bool) -> Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    // StartupApproved\Run 键由 Windows 创建；可能不存在，需先确保
    let (approved, _) = hkcu.create_subkey(APPROVED_KEY_PATH)?;
    let mut bytes = [0u8; 12];
    bytes[0] = if enable { 0x02 } else { 0x03 };
    let value = winreg::RegValue {
        bytes: bytes.to_vec(),
        vtype: REG_BINARY,
    };
    approved.set_raw_value(VALUE_NAME, &value)?;
    Ok(())
}

/// 设置开机自启。
///
/// 启用：写 Run 键 `"<root>\ClashEdge.exe" --clash-edge-autostart`，并写
/// StartupApproved=启用，避免任务管理器的旧禁用状态覆盖。
/// 禁用：删除 Run 值，并把 StartupApproved 置为禁用。
pub fn set_autostart(enable: bool) -> Result<()> {
    let launcher = root_launcher_path();
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let run = hkcu.open_subkey_with_flags(RUN_KEY_PATH, KEY_READ | KEY_WRITE)?;

    if enable {
        let value = format!("\"{}\" {}", launcher.display(), AUTOSTART_ARG);
        run.set_value(VALUE_NAME, &value)?;
        set_approved_state(true)?;
        tracing::info!("Autostart enabled via {:?}", value);
    } else {
        match run.delete_value(VALUE_NAME) {
            Ok(()) => {}
            // 值不存在视为已禁用，不报错
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(Error::Io(e)),
        }
        set_approved_state(false)?;
        tracing::info!("Autostart disabled");
    }
    Ok(())
}

/// 是否为浅色任务栏（任务栏背景是浅色）。
/// 读 `HKCU\...\Themes\Personalize\AppsUseLightTheme` DWORD：1=浅色，0=深色。
/// 读不到时返回 `false`（深色任务栏上深色图标反而看不清，这里返回浅色/深色的
/// 配对由调用方决定；缺省视为深色，最稳妥）。实际语义：返回 true 表示「任务栏是浅色」。
pub fn is_light_taskbar() -> bool {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key = hkcu
        .open_subkey_with_flags(
            r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
            KEY_READ,
        )
        .ok();
    let Some(key) = key else {
        return false;
    };
    match key.get_value::<u32, _>("AppsUseLightTheme") {
        Ok(v) => v == 1,
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn launcher_path(value: &str) -> Option<PathBuf> {
        parse_launcher_path(std::ffi::OsStr::new(value))
    }

    #[test]
    fn parses_quoted_path_with_args() {
        let p = launcher_path(r#""D:\Apps\ClashEdge Port\ClashEdge.exe" --clash-edge-autostart"#);
        assert_eq!(p, Some(PathBuf::from(r"D:\Apps\ClashEdge Port\ClashEdge.exe")));
    }

    #[test]
    fn parses_quoted_path_with_unicode_and_spaces() {
        let p = launcher_path(r#""D:\工具\ClashEdge 便携版\ClashEdge.exe" --clash-edge-autostart"#);
        assert_eq!(
            p,
            Some(PathBuf::from(r"D:\工具\ClashEdge 便携版\ClashEdge.exe"))
        );
    }

    #[test]
    fn parses_bare_path_with_args() {
        let p = launcher_path(r"C:\Portable\ClashEdge.exe --clash-edge-autostart");
        assert_eq!(p, Some(PathBuf::from(r"C:\Portable\ClashEdge.exe")));
    }

    #[test]
    fn parses_quoted_path_trailing_quote_with_spaces() {
        // 路径含空格但带引号：必须完整解析
        let p = launcher_path(r#""D:\My Folder\ClashEdge.exe"--clash-edge-autostart"#);
        assert_eq!(p, Some(PathBuf::from(r"D:\My Folder\ClashEdge.exe")));
    }

    #[test]
    fn rejects_empty_quotes() {
        assert_eq!(launcher_path(r#""""#), None);
    }

    #[test]
    fn rejects_unclosed_quote() {
        assert_eq!(launcher_path(r#""D:\foo"#), None);
    }

    #[test]
    fn rejects_empty() {
        assert_eq!(launcher_path(""), None);
        assert_eq!(launcher_path("   "), None);
    }
}
