// src-tauri/src/util/atomic.rs
//! 原子写入工具：随机后缀临时文件 + 排他创建 + rename
//!
//! 低危修复：把 `*.tmp` 固定名改为「进程 id + 计数器」随机后缀，并用
//! `OpenOptions::create_new(true)` 排他创建，避免并发写入 / 崩溃残留的
//! 同名临时文件互相踩踏。config.yaml 与 runtime-config.yaml、导出配置共用。

use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::util::error::Result;

/// 临时文件名计数器（与进程 id 组合成随机后缀，保证同一进程内不撞名）
static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// 原子写入文件：先写带随机后缀的临时文件（`create_new` 排他创建），
/// 再 rename 到目标路径（同文件系统内原子替换）。
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = temp_path_for(path);
    {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        file.write_all(bytes)?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// 生成临时文件路径：`{path}.tmp.{pid}-{n}`（随机后缀，与目标同目录同文件系统）。
fn temp_path_for(path: &Path) -> std::path::PathBuf {
    let n = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".tmp.{}-{}", std::process::id(), n));
    path.with_file_name(name)
}
