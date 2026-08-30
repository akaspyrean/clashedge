// src-tauri/src/commands/profiles/files.rs
//! profile 文件操作辅助：净化路径构造、临时/备份/待删路径、
//! 事务式文件替换（commit_profile_file）、激活失败回滚（activate_with_rollback）。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use tauri::{AppHandle, Manager};
use tracing::warn;

use crate::util::error::{Error, Result};
use crate::util::paths::sanitize_profile_name;

/// 临时文件名计数器（与进程 id 组合成随机后缀，保证同一进程内不撞名）
static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// 构造净化后的 profile 文件路径（所有 profile 命令统一入口）
pub(super) fn profile_path(
    profiles_dir: &std::path::Path,
    name: &str,
) -> Result<std::path::PathBuf> {
    let safe = sanitize_profile_name(name)?;
    Ok(profiles_dir.join(format!("{}.yaml", safe)))
}

/// 生成临时文件路径：`{path}.tmp.{pid}-{n}`（随机后缀，与目标同目录同文件系统）。
pub(super) fn temp_path_for(path: &Path) -> PathBuf {
    let n = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".tmp.{}-{}", std::process::id(), n));
    path.with_file_name(name)
}

/// 生成备份文件路径：`{name}.yaml` -> `{name}.yaml.bak`
pub(super) fn backup_path_for(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".bak");
    path.with_file_name(name)
}

/// 删除暂存路径：`{name}.yaml` -> `{name}.yaml.pending-delete`。
/// 扩展名不再是 `.yaml`，不会被 `list_profiles` 扫描到，删除中途失败时文件可恢复。
pub(super) fn pending_delete_path_for(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".pending-delete");
    path.with_file_name(name)
}

/// 当前激活的 profile 名（来自共享配置）
pub(super) fn active_profile(app: &AppHandle) -> String {
    app.state::<crate::AppState>()
        .config_manager
        .lock()
        .unwrap()
        .get_config()
        .general
        .profile
}

/// 事务式用临时文件替换正式 profile 文件：
/// 1. 若旧文件存在，先重命名为 `{name}.yaml.bak`（残留旧备份先清理）；
/// 2. 临时文件 rename 为正式文件；
/// 3. 任一步失败：把 .bak 恢复原位、清理临时文件，返回 Err——
///    保证原来能工作的订阅不会因半途失败而丢失。
pub(super) fn commit_profile_file(temp_path: &Path, target: &Path) -> Result<()> {
    let backup = backup_path_for(target);

    if target.exists() {
        let _ = std::fs::remove_file(&backup);
        std::fs::rename(target, &backup)?;
    }

    if let Err(e) = std::fs::rename(temp_path, target) {
        if backup.exists() && !target.exists() {
            let _ = std::fs::rename(&backup, target);
        }
        let _ = std::fs::remove_file(temp_path);
        return Err(e.into());
    }
    Ok(())
}

/// 激活 profile 并使其生效；激活失败时回滚已替换的文件（用 `.bak` 恢复旧内容）
/// 并重新激活旧内容，保证「文件 + config + 运行核心」三者一致。
///
/// 前置：调用方已通过 `commit_profile_file` 把旧文件备份到 `.bak`、新内容写到
/// target，且**已持有配置事务锁**（本函数用 `activate_profile_locked`，不再取
/// 锁，避免嵌套死锁）。若激活（重启核心加载新内容）失败，target 上是坏的新版本，
/// `.bak` 里是仍能工作的旧版本——这里恢复旧版本并重新激活，避免"磁盘已是坏新版本、
/// 运行仍是旧状态"的半套状态。
pub(super) async fn activate_with_rollback(
    app: &AppHandle,
    name: &str,
    file_path: &Path,
) -> Result<()> {
    if let Err(e) = crate::core::runtime::activate_profile_locked(app, name).await {
        // 回滚：用 .bak 恢复旧内容。文件操作必须确认真的成功——若恢复也失败，
        // 要如实上报"操作失败且自动恢复失败"，并保留 backup 供手工恢复，而不是
        // 谎称"已回滚"。rename 在 Windows 上可替换已存在目标；失败则先删再 rename。
        let backup = backup_path_for(file_path);
        let restore_ok = std::fs::rename(&backup, file_path)
            .or_else(|_| {
                let _ = std::fs::remove_file(file_path);
                std::fs::rename(&backup, file_path)
            })
            .is_ok();

        if restore_ok {
            if let Err(e2) = crate::core::runtime::activate_profile_locked(app, name).await {
                warn!("Rollback re-activate failed for '{}': {}", name, e2);
            }
            return Err(Error::Other(format!(
                "Profile '{}' 保存生效失败，已回滚到旧内容：{}",
                name, e
            )));
        }
        // 文件恢复失败：保留 backup，明确提示备份位置，绝不静默吞掉。
        warn!(
            "Profile '{}' rollback file restore FAILED; backup preserved at {}",
            name,
            backup.display()
        );
        return Err(Error::Other(format!(
            "Profile '{}' 保存生效失败，且自动恢复文件也失败（备份保留在 {}）：{}",
            name,
            backup.display(),
            e
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(std::path::PathBuf);
    impl TempDir {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "clashedge-profiles-test-{}-{}-{}",
                tag,
                std::process::id(),
                TMP_COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }
        fn path(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn temp_paths_are_unique_and_suffixed() {
        let dir = TempDir::new("tmpname");
        let target = dir.path("sub.yaml");
        let a = temp_path_for(&target);
        let b = temp_path_for(&target);
        assert_ne!(a, b);
        let name = a.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with("sub.yaml.tmp."));
        assert!(a.parent() == target.parent(), "temp 必须与目标同目录");
    }

    #[test]
    fn commit_replaces_file_and_keeps_backup() {
        let dir = TempDir::new("commit-ok");
        let target = dir.path("sub.yaml");
        std::fs::write(&target, b"old content").unwrap();
        let temp = temp_path_for(&target);
        std::fs::write(&temp, b"new content").unwrap();

        commit_profile_file(&temp, &target).unwrap();

        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new content");
        // 旧内容保留在 .bak，临时文件已被消费（rename 走）不再存在
        assert_eq!(
            std::fs::read_to_string(backup_path_for(&target)).unwrap(),
            "old content"
        );
        assert!(!temp.exists());
    }

    #[test]
    fn commit_failure_restores_backup_and_cleans_temp() {
        let dir = TempDir::new("rollback");
        let target = dir.path("sub.yaml");
        std::fs::write(&target, b"original").unwrap();
        // 临时文件不存在 → 第二步 rename 必然失败，触发回滚
        let temp = temp_path_for(&target);

        assert!(commit_profile_file(&temp, &target).is_err());

        // .bak 已恢复原位：原订阅内容完好，无残留备份/临时文件
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "original");
        assert!(!backup_path_for(&target).exists());
        assert!(!temp.exists());
    }
}
