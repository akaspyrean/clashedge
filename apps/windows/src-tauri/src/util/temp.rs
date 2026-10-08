// src-tauri/src/util/temp.rs
//! 临时文件 RAII 守卫（审计 B2）：离开作用域时自动删除，
//! 杜绝"下载/归一化中途 `?` 返回导致 `.tmp.*` 文件堆积"。

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// 持有临时文件路径；`Drop` 时尽力删除该路径。
///
/// 成功提交的流程会先把临时文件 rename 到正式位置，此时 Drop 面对的是一个已不存在的
/// 路径，删除是无害的空操作；任何提前返回（含 `?`）则会清理残留文件。
pub struct TempFile {
    path: PathBuf,
}

impl TempFile {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// 清理目录下的陈旧残留：文件名含 `.tmp.` 或以 `.pending-delete` 结尾，
/// 且修改时间早于 `max_age` 的文件。启动时调用一次即可。
pub fn sweep_stale(dir: &Path, max_age: Duration) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let now = SystemTime::now();
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !(name.contains(".tmp.") || name.ends_with(".pending-delete")) {
            continue;
        }
        let old_enough = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| now.duration_since(t).ok())
            .map(|d| d >= max_age)
            .unwrap_or(false);
        if old_enough && std::fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("clashedge-temp-{}-{}", tag, std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn drop_removes_uncommitted_file() {
        let d = scratch("drop");
        let p = d.join("a.tmp.1-0");
        std::fs::write(&p, b"x").unwrap();
        {
            let _g = TempFile::new(p.clone());
        }
        assert!(!p.exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn sweep_only_removes_old_tmp_files() {
        let d = scratch("sweep");
        let tmp = d.join("sub.yaml.tmp.9-1");
        let keep = d.join("sub.yaml");
        std::fs::write(&tmp, b"x").unwrap();
        std::fs::write(&keep, b"y").unwrap();
        // max_age=0 → 一切 tmp 都"足够旧"
        assert_eq!(sweep_stale(&d, Duration::ZERO), 1);
        assert!(!tmp.exists());
        assert!(keep.exists());
        // 很长的 max_age → 不删
        std::fs::write(&tmp, b"x").unwrap();
        assert_eq!(sweep_stale(&d, Duration::from_secs(3600)), 0);
        let _ = std::fs::remove_dir_all(&d);
    }
}
