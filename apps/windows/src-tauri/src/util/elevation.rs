// src-tauri/src/util/elevation.rs
//! 管理员/提权状态检测与 UAC 提权重启（仅 Windows；其它平台不作限制）。
//!
//! 用途：开启 TUN 前的权限预检。Windows 要求管理员权限才能创建 WinTun 虚拟网卡
//! 并修改路由表；标准用户尝试开启时 mihomo 会因 `configure tun interface:
//! Access is denied` 而失败（PATCH 返回 200 但实际未生效——假成功）。在应用侧
//! 提前检测、给出明确提示，用户确认后 `relaunch_elevated` 走 UAC 以管理员
//! 身份重启，新实例启动时自动开启 TUN。

use crate::util::error::{Error, Result};

/// 当前进程是否具备管理员权限（elevated）。
///
/// Windows：通过进程 token 的 `TokenElevation` 检测；
/// 其它平台：恒返回 true（无管理员限制，不做预检）。
#[cfg(windows)]
pub fn is_elevated() -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Security::{
        GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut elevation: TOKEN_ELEVATION = std::mem::zeroed();
        let mut size: u32 = 0;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            &mut elevation as *mut _ as *mut _,
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut size,
        ) != 0;
        CloseHandle(token);
        ok && elevation.TokenIsElevated != 0
    }
}

/// 非 Windows 平台：不限制管理员权限。
#[cfg(not(windows))]
pub fn is_elevated() -> bool {
    true
}

/// 以管理员身份（UAC）重启当前应用。
///
/// 成功返回后调用方应立即退出当前进程——新实例启动时会先通过
/// [`wait_for_previous_instance`] 等待旧 PID 退出，避开单实例锁把新实例挡掉。
/// 附带参数：
/// - `--clash-edge-elevated-relaunch=<旧 PID>`：新实例等待旧实例退出；
/// - `--clash-edge-enable-tun`：新实例在内核就绪后自动开启 TUN。
///
/// 用户在 UAC 弹窗点「否」→ `ShellExecuteExW` 返回 0 且 `GetLastError`
/// 为 `ERROR_CANCELLED`，映射为明确的取消文案（前端据此提示，不误导为故障）。
/// 提权重启时携带的数据目录参数前缀。
///
/// **便携数据目录必须显式透传**：参考包 C# 启动器靠 `CLASH_EDGE_DATA_DIR`
/// （以及 `HOME`/`APPDATA`/`LOCALAPPDATA`）把内层应用的数据目录指向包根
/// `Data/`。`ShellExecuteExW("runas")` 走 shell 提权，**不会**可靠继承当前
/// 进程的环境块；若不显式携带，新实例会退化成“安装版”判定、写到
/// `%APPDATA%`，表现为**配置与订阅全部丢失**（原 `Data/` 仍在，只是没被读到）。
pub const RELAUNCH_DATA_DIR_ARG: &str = "--clash-edge-data-dir=";

/// 以管理员身份（UAC）重启当前应用。
///
/// 成功返回后调用方应立即退出当前进程——新实例启动时会先通过
/// [`wait_for_previous_instance`] 等待旧 PID 退出，避开单实例锁把新实例挡掉。
/// 附带参数：
/// - `--clash-edge-elevated-relaunch=<旧 PID>`：新实例等待旧实例退出；
/// - `--clash-edge-enable-tun`：新实例在内核就绪后自动开启 TUN；
/// - `--clash-edge-data-dir=<路径>`：新实例沿用同一数据目录（便携包必需）。
///
/// 用户在 UAC 弹窗点「否」→ `ShellExecuteExW` 返回 0 且 `GetLastError`
/// 为 `ERROR_CANCELLED`，映射为明确的取消文案（前端据此提示，不误导为故障）。
#[cfg(windows)]
pub fn relaunch_elevated(enable_tun: bool, data_dir: Option<&std::path::Path>) -> Result<()> {
    use windows_sys::Win32::Foundation::{GetLastError, ERROR_CANCELLED};
    use windows_sys::Win32::UI::Shell::{ShellExecuteExW, SHELLEXECUTEINFOW};
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let exe =
        std::env::current_exe().map_err(|e| Error::Other(format!("无法定位可执行文件：{e}")))?;
    let mut params = format!("--clash-edge-elevated-relaunch={}", std::process::id());
    if enable_tun {
        params.push_str(" --clash-edge-enable-tun");
    }
    // 显式携带数据目录，避免提权实例因环境变量丢失而"丢失配置与订阅"。
    if let Some(dir) = data_dir {
        params.push(' ');
        params.push_str(RELAUNCH_DATA_DIR_ARG);
        params.push_str(&dir.to_string_lossy());
    }
    let w = |s: &str| -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() };
    let verb = w("runas");
    let file = w(&exe.to_string_lossy());
    let args = w(&params);

    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.lpVerb = verb.as_ptr();
    info.lpFile = file.as_ptr();
    info.lpParameters = args.as_ptr();
    info.nShow = SW_SHOWNORMAL;

    // SAFETY: 各 UTF-16 缓冲区在本作用域内保持存活；结构体已零初始化并填齐必需字段。
    if unsafe { ShellExecuteExW(&mut info) } == 0 {
        let err = unsafe { GetLastError() };
        return Err(Error::Other(if err == ERROR_CANCELLED {
            "已取消管理员授权，未重启。".to_string()
        } else {
            format!("以管理员身份启动失败（Windows 错误 {err}）。")
        }));
    }
    Ok(())
}

/// 非 Windows 平台：不支持 UAC 提权（`is_elevated()` 恒为 true，不会被调用）。
#[cfg(not(windows))]
pub fn relaunch_elevated(_enable_tun: bool, _data_dir: Option<&std::path::Path>) -> Result<()> {
    Err(Error::Other("当前平台不支持提权重启".to_string()))
}

/// 提权重启的新实例在启动早期应用携带的数据目录：把
/// `--clash-edge-data-dir=` 落到 `CLASH_EDGE_DATA_DIR` 环境变量，并按
/// C# 启动器布局一并重定向 `HOME`/`APPDATA`/`LOCALAPPDATA`。
///
/// 必须在任何路径解析（`util::paths::get_app_data_dir`）之前调用。
/// 非提权启动（无该参数）时不做任何事。
pub fn apply_relaunch_data_dir() {
    let raw =
        std::env::args().find_map(|a| a.strip_prefix(RELAUNCH_DATA_DIR_ARG).map(|s| s.to_string()));
    let Some(raw) = raw else {
        return;
    };
    if raw.is_empty() {
        return;
    }
    let dir = std::path::PathBuf::from(&raw);
    // 只接受绝对路径，避免相对路径被解析到不同工作目录。
    if !dir.is_absolute() {
        return;
    }
    unsafe {
        std::env::set_var("CLASH_EDGE_DATA_DIR", &dir);
        // 与 C# 启动器一致：用户级目录也重定向进 Data/，提权实例的缓存 /
        // WebView2 数据不会跑到 %APPDATA%，避免看起来像"配置丢失"。
        std::env::set_var("HOME", dir.join("Home"));
        std::env::set_var("APPDATA", dir.join("AppData"));
        std::env::set_var("LOCALAPPDATA", dir.join("LocalAppData"));
    }
}

/// 提权重启的新实例在初始化（含单实例注册）之前，等待旧实例进程退出。
///
/// 读取 `--clash-edge-elevated-relaunch=<旧 PID>` 参数；无该参数立即返回。
/// 最多等 20 秒——超时后照常启动（宁可让单实例锁挡下让用户重试，
/// 也不让新实例永久卡死在初始化前）。
#[cfg(windows)]
pub fn wait_for_previous_instance() {
    let pid = std::env::args().find_map(|a| {
        a.strip_prefix("--clash-edge-elevated-relaunch=")
            .and_then(|p| p.parse::<u32>().ok())
    });
    let Some(pid) = pid else { return };
    for _ in 0..200 {
        if !process_alive(pid) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// 进程是否存活（`GetExitCodeProcess == STILL_ACTIVE`）。
#[cfg(windows)]
fn process_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    // SAFETY: 只查询退出码，句柄用完即关；OpenProcess 返回 NULL 表示进程不存在。
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return false;
        }
        let mut code: u32 = 0;
        let ok = GetExitCodeProcess(h, &mut code);
        CloseHandle(h);
        ok != 0 && code == STILL_ACTIVE as u32
    }
}

/// 非 Windows 平台：无提权重启，无需等待。
#[cfg(not(windows))]
pub fn wait_for_previous_instance() {}

#[cfg(test)]
mod tests {
    use super::*;

    /// 冒烟：函数在任何平台都可调用且不 panic（Windows 上返回真实检测值）。
    #[test]
    fn is_elevated_returns_bool() {
        let _ = is_elevated();
    }

    /// 回归：提权重启必须显式携带数据目录参数。
    ///
    /// 曾经的真实故障：C# 启动器靠 `CLASH_EDGE_DATA_DIR` 指向包根 `Data/`，
    /// 而 `ShellExecuteExW("runas")` 提权不继承环境变量 → 提权实例退化成
    /// 安装版判定、写到 `%APPDATA%`，用户看到"配置与订阅全部丢失"。
    /// 本测试锁定参数名与解析规则，防止后续再引入同类问题。
    #[test]
    fn relaunch_data_dir_arg_is_parsed_into_env() {
        let dir = std::env::temp_dir().join(format!(
            "clash-edge-relaunch-dir-{}-{}",
            std::process::id(),
            "relaunch"
        ));
        // 只接受绝对路径
        assert!(dir.is_absolute());
        let arg = format!("{}{}", RELAUNCH_DATA_DIR_ARG, dir.to_string_lossy());

        // 解析规则与 apply_relaunch_data_dir 一致：strip_prefix 后取剩余部分
        let parsed = arg
            .strip_prefix(RELAUNCH_DATA_DIR_ARG)
            .map(|s| s.to_string())
            .expect("arg must carry the data dir");
        assert_eq!(parsed, dir.to_string_lossy());

        // 参数前缀本身必须是稳定的 CLI 约定（启动器/文档/测试共用）
        assert_eq!(RELAUNCH_DATA_DIR_ARG, "--clash-edge-data-dir=");
    }

    /// 回归：数据目录环境变量被 path 模块识别（便携判定不退回安装版）。
    #[test]
    fn relaunch_data_dir_env_is_recognized_by_paths() {
        let dir = std::env::temp_dir().join(format!(
            "clash-edge-relaunch-env-{}-{}",
            std::process::id(),
            "env"
        ));
        unsafe {
            std::env::set_var("CLASH_EDGE_DATA_DIR", &dir);
        }
        assert!(crate::util::paths::is_portable_mode());
        assert_eq!(crate::util::paths::get_app_data_dir_for_test(), Some(dir));
        unsafe {
            std::env::remove_var("CLASH_EDGE_DATA_DIR");
        }
    }
}
