// src-tauri/src/util/process.rs
//! 进程辅助：映像路径所有权校验、持句柄终止、监听端口占用者查询。
//!
//! 设计要点（审计 A1 / C2）：
//! - 终止进程不再依赖 `taskkill`：它没有 `CREATE_NO_WINDOW`（退出时闪控制台），
//!   且「校验映像路径」与「按 PID 终止」是两步，PID 在两步之间可能被复用。
//!   这里先 `OpenProcess` 取得句柄，用**同一个句柄**校验映像路径再
//!   `TerminateProcess`，消除复用窗口；
//! - 任何一步失败一律 fail-closed（返回 false / Err），绝不误杀无关进程。

use std::path::{Path, PathBuf};

use crate::util::error::{Error, Result};

/// 路径归一化比较键：canonicalize（失败则原样）→ 去 verbatim 前缀 → 大写。
pub fn path_key(p: &Path) -> String {
    let canon = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let s = canon.to_string_lossy().to_string();
    let s = s
        .strip_prefix(r"\\?\")
        .or_else(|| s.strip_prefix(r"\??\"))
        .unwrap_or(&s)
        .to_string();
    s.to_uppercase()
}

/// 两个路径是否指向同一文件（大小写不敏感）。
pub fn same_path(a: &Path, b: &Path) -> bool {
    path_key(a) == path_key(b)
}

#[cfg(target_os = "windows")]
mod imp {
    use super::*;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GetExtendedTcpTable, MIB_TCPROW_OWNER_PID, MIB_TCPTABLE_OWNER_PID,
        TCP_TABLE_OWNER_PID_LISTENER,
    };
    use windows_sys::Win32::Networking::WinSock::AF_INET;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, TerminateProcess, WaitForSingleObject,
        PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
    };

    /// 句柄 RAII，保证任何分支都会 CloseHandle。
    struct OwnedHandle(HANDLE);
    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    CloseHandle(self.0);
                }
            }
        }
    }

    fn open(pid: u32, access: u32) -> Option<OwnedHandle> {
        let h = unsafe { OpenProcess(access, 0, pid) };
        if h.is_null() {
            None
        } else {
            Some(OwnedHandle(h))
        }
    }

    fn image_of_handle(h: &OwnedHandle) -> Option<PathBuf> {
        let mut buf = [0u16; 1024];
        let mut size = buf.len() as u32;
        let ok = unsafe { QueryFullProcessImageNameW(h.0, 0, buf.as_mut_ptr(), &mut size) };
        if ok == 0 {
            return None;
        }
        Some(PathBuf::from(String::from_utf16_lossy(
            &buf[..size as usize],
        )))
    }

    pub fn image_path(pid: u32) -> Option<PathBuf> {
        let h = open(pid, PROCESS_QUERY_LIMITED_INFORMATION)?;
        image_of_handle(&h)
    }

    pub fn terminate_owned(pid: u32, expected: &Path, wait_ms: u32) -> Result<bool> {
        let Some(h) = open(
            pid,
            PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
        ) else {
            // 进程已不存在或无权限：视为"不属于我们 / 已退出"，不报错也不杀。
            return Ok(false);
        };
        let Some(image) = image_of_handle(&h) else {
            return Ok(false);
        };
        if !same_path(&image, expected) {
            return Err(Error::Other(format!(
                "refusing to terminate PID {}: image '{}' is not '{}'",
                pid,
                image.display(),
                expected.display()
            )));
        }
        let ok = unsafe { TerminateProcess(h.0, 1) };
        if ok == 0 {
            return Err(Error::Other(format!(
                "TerminateProcess failed for PID {}",
                pid
            )));
        }
        let waited = unsafe { WaitForSingleObject(h.0, wait_ms) };
        Ok(waited == WAIT_OBJECT_0)
    }

    /// 查询监听在 `port`（IPv4）上的进程 PID。
    pub fn port_owner_pid(port: u16) -> Option<u32> {
        let mut size: u32 = 0;
        // 第一次调用取所需缓冲区大小（返回 ERROR_INSUFFICIENT_BUFFER）。
        unsafe {
            GetExtendedTcpTable(
                std::ptr::null_mut(),
                &mut size,
                0,
                AF_INET as u32,
                TCP_TABLE_OWNER_PID_LISTENER,
                0,
            );
        }
        if size == 0 {
            return None;
        }
        // 用 u64 缓冲保证 8 字节对齐。
        let mut buf = vec![0u64; (size as usize).div_ceil(8) + 1];
        let rc = unsafe {
            GetExtendedTcpTable(
                buf.as_mut_ptr() as *mut _,
                &mut size,
                0,
                AF_INET as u32,
                TCP_TABLE_OWNER_PID_LISTENER,
                0,
            )
        };
        if rc != 0 {
            return None;
        }
        let table = buf.as_ptr() as *const MIB_TCPTABLE_OWNER_PID;
        unsafe {
            let n = (*table).dwNumEntries as usize;
            let rows: &[MIB_TCPROW_OWNER_PID] =
                std::slice::from_raw_parts((*table).table.as_ptr(), n);
            rows.iter()
                .find(|r| u16::from_be((r.dwLocalPort & 0xFFFF) as u16) == port)
                .map(|r| r.dwOwningPid)
        }
    }
}

#[cfg(not(target_os = "windows"))]
mod imp {
    use super::*;
    pub fn image_path(_pid: u32) -> Option<PathBuf> {
        None
    }
    pub fn terminate_owned(_pid: u32, _expected: &Path, _wait_ms: u32) -> Result<bool> {
        Ok(false)
    }
    pub fn port_owner_pid(_port: u16) -> Option<u32> {
        None
    }
}

/// 该 PID 的映像路径是否确认为 `expected`。任何环节失败返回 false（fail-closed）。
pub fn owns_pid(pid: u32, expected: Option<&Path>) -> bool {
    let Some(expected) = expected else {
        return false;
    };
    match imp::image_path(pid) {
        Some(image) => same_path(&image, expected),
        None => false,
    }
}

/// 持句柄校验映像路径后终止进程，并等待至多 `wait_ms` 毫秒确认退出。
/// - `Ok(true)`：已终止并确认退出；
/// - `Ok(false)`：进程不存在 / 无权限 / 终止后未在期限内退出；
/// - `Err`：映像路径不匹配（拒绝终止）或 TerminateProcess 失败。
pub fn terminate_owned(pid: u32, expected: &Path, wait_ms: u32) -> Result<bool> {
    imp::terminate_owned(pid, expected, wait_ms)
}

/// 监听在 `port` 上的进程：`(pid, 映像文件名)`。
pub fn port_owner(port: u16) -> Option<(u32, String)> {
    let pid = imp::port_owner_pid(port)?;
    let name = imp::image_path(pid)
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_else(|| "unknown".to_string());
    Some((pid, name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_key_is_case_and_prefix_insensitive() {
        let a = Path::new(r"C:\Windows\System32\cmd.exe");
        let b = Path::new(r"c:\windows\system32\CMD.EXE");
        assert!(same_path(a, b));
        assert!(!same_path(a, Path::new(r"C:\Windows\notepad.exe")));
    }

    #[test]
    fn owns_pid_fails_closed() {
        // 没有期望路径 → false
        assert!(!owns_pid(std::process::id(), None));
        // 不存在的 PID → false
        assert!(!owns_pid(u32::MAX - 7, Some(Path::new(r"C:\x.exe"))));
        // 自身 PID 但期望路径不是自身 → false
        assert!(!owns_pid(
            std::process::id(),
            Some(Path::new(r"C:\definitely\not\me.exe"))
        ));
    }

    #[test]
    fn terminate_refuses_foreign_image() {
        // 对自身 PID 声称另一个映像路径：必须拒绝（Err），且当前进程仍存活。
        let r = terminate_owned(std::process::id(), Path::new(r"C:\not\me.exe"), 10);
        #[cfg(target_os = "windows")]
        assert!(r.is_err());
        #[cfg(not(target_os = "windows"))]
        assert!(matches!(r, Ok(false)));
    }

    #[test]
    fn port_owner_finds_our_listener() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        #[cfg(target_os = "windows")]
        {
            let owner = port_owner(port).expect("listener owner must be found");
            assert_eq!(owner.0, std::process::id());
        }
        drop(l);
        let _ = port;
    }
}
