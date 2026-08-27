// src-tauri/src/util/elevation.rs
//! 管理员/提权状态检测（仅 Windows；其它平台恒为"已提权"，不作限制）。
//!
//! 用途：开启 TUN 前的权限预检。Windows 要求管理员权限才能创建 WinTun 虚拟网卡
//! 并修改路由表；标准用户尝试开启时 mihomo 会因 `configure tun interface:
//! Access is denied` 而失败（PATCH 返回 200 但实际未生效——假成功）。在应用侧
//! 提前检测、给出明确提示，避免用户反复开启却莫名失败。

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

#[cfg(test)]
mod tests {
    use super::*;

    /// 冒烟：函数在任何平台都可调用且不 panic（Windows 上返回真实检测值）。
    #[test]
    fn is_elevated_returns_bool() {
        let _ = is_elevated();
    }
}
