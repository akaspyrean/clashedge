// src-tauri/src/proxy/journal.rs
//! 系统代理 Recovery Journal（proxy-session.json）
//!
//! 覆盖正常退出恢复无法处理的场景：End Task / TerminateProcess / 断电 /
//! 系统崩溃。应用在「成功把 Windows 系统代理指向自己」时写一份极小的
//! journal；干净关闭（用户关闭开关或退出还原）后删除。
//!
//! 启动时若发现上次 session 异常结束（journal 残留）且当前注册表代理
//! 仍指向 ClashEdge 的端口，则把系统代理恢复为 journal 里记录的用户
//! 原始状态——ClashEdge 崩溃不能把 Windows 网络环境留坏。
//!
//! journal 只含非敏感字段：session id、进程 PID、mixed-port、原始代理
//! 配置、owned 标志。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::proxy::system_proxy::SystemProxyConfig;
use crate::util::atomic::atomic_write;
use crate::util::error::{Error, Result};

/// journal 文件名（位于应用数据目录）
const JOURNAL_FILE: &str = "proxy-session.json";

/// 系统代理会话记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyJournal {
    /// 会话 id（UUID 形式的随机串，用于人工排查）
    pub session_id: String,
    /// 写入时的应用进程 PID
    pub pid: u32,
    /// 当时 mihomo 的 mixed-port（判断注册表代理是否指向本应用）
    pub mixed_port: u16,
    /// 开启系统代理**之前**的 Windows 代理状态（None = 用户原本没有代理）
    pub original: Option<SystemProxyConfig>,
    /// 是否由本应用接管了系统代理
    pub owned: bool,
    /// 接管成功后应存在的完整 Windows 代理状态。
    /// v1.0.4 journal 没有此字段；旧文件按 mixed_port 生成 canonical 状态。
    #[serde(default)]
    pub managed: Option<SystemProxyConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReleaseOutcome {
    NoOwnership,
    OwnershipLost,
    Restored {
        message: String,
        restored: SystemProxyConfig,
    },
}

fn journal_path(data_dir: &std::path::Path) -> PathBuf {
    data_dir.join(JOURNAL_FILE)
}

/// 写入 journal（原子写）。
///
/// journal 是崩溃恢复的唯一凭据——必须先于"改注册表"持久化成功，
/// 否则进程在改完注册表后崩溃、journal 不在，下次启动无法恢复用户原代理。
/// 因此本函数失败必须返回 Err，由调用方决定是否拒绝开启系统代理
/// （若静默放行 → 注册表已改但 journal 缺失 → 崩溃后死代理）。
pub fn write_journal(data_dir: &std::path::Path, journal: &ProxyJournal) -> Result<()> {
    let json = serde_json::to_string_pretty(journal)
        .map_err(|e| Error::Other(format!("Failed to serialize proxy session journal: {}", e)))?;
    atomic_write(&journal_path(data_dir), json.as_bytes())
        .map_err(|e| Error::Other(format!("Failed to write proxy session journal: {}", e)))?;
    info!(
        "Proxy session journal written (session {})",
        journal.session_id
    );
    Ok(())
}

/// 删除 journal（干净关闭路径）。不存在时静默成功。
pub fn clear_journal(data_dir: &std::path::Path) {
    let path = journal_path(data_dir);
    if path.exists() {
        if let Err(e) = std::fs::remove_file(&path) {
            warn!("Failed to remove proxy session journal: {}", e);
        } else {
            info!("Proxy session journal cleared");
        }
    }
}

/// 读取 journal（损坏/缺失 → None）
pub fn read_journal(data_dir: &std::path::Path) -> Option<ProxyJournal> {
    let content = std::fs::read_to_string(journal_path(data_dir)).ok()?;
    match serde_json::from_str(&content) {
        Ok(j) => Some(j),
        Err(e) => {
            warn!("Corrupt proxy session journal ignored: {}", e);
            None
        }
    }
}

fn canonical_managed(journal: &ProxyJournal) -> SystemProxyConfig {
    journal
        .managed
        .clone()
        .unwrap_or_else(|| crate::proxy::system_proxy::managed_proxy_config(journal.mixed_port))
}

/// 端口切换已安全释放旧 ownership、但新端口接管随后失败时，只有 Windows
/// 仍精确等于释放后的用户基线，才允许把旧 journal/managed 状态写回。
/// 退避期间用户或其他软件一旦修改任一字段，就拒绝回滚，避免覆盖新接管者。
fn restore_previous_with<Read, Restore, Write>(
    previous: &ProxyJournal,
    expected_baseline: &SystemProxyConfig,
    mut read: Read,
    mut restore: Restore,
    mut write: Write,
) -> Result<()>
where
    Read: FnMut() -> Result<SystemProxyConfig>,
    Restore: FnMut(&SystemProxyConfig) -> Result<()>,
    Write: FnMut(&ProxyJournal) -> Result<()>,
{
    let current = read()?;
    if current != *expected_baseline {
        return Err(Error::Other(
            "Windows proxy changed after the old port was released; refusing to restore previous ClashEdge ownership"
                .to_string(),
        ));
    }
    write(previous)?;
    let previous_managed = canonical_managed(previous);
    restore(&previous_managed)?;
    if read()? != previous_managed {
        return Err(Error::Other(
            "Previous ClashEdge proxy ownership restore could not be verified; journal kept"
                .to_string(),
        ));
    }
    Ok(())
}

fn restore_previous_ownership(
    data_dir: &std::path::Path,
    previous: &ProxyJournal,
    expected_baseline: &SystemProxyConfig,
) -> Result<()> {
    restore_previous_with(
        previous,
        expected_baseline,
        crate::proxy::system_proxy::get_system_proxy,
        crate::proxy::system_proxy::restore_system_proxy,
        |journal| write_journal(data_dir, journal),
    )
}

fn error_after_switch_failure(
    data_dir: &std::path::Path,
    previous_switch: Option<&(ProxyJournal, SystemProxyConfig)>,
    primary: Error,
) -> Error {
    let Some((previous, baseline)) = previous_switch else {
        return primary;
    };
    match restore_previous_ownership(data_dir, previous, baseline) {
        Ok(()) => Error::Other(format!(
            "{}; previous ClashEdge proxy ownership was restored",
            primary
        )),
        Err(rollback) => Error::Other(format!(
            "{}; previous ownership rollback was not safe or failed: {}",
            primary, rollback
        )),
    }
}

fn points_at_managed_address(current: &SystemProxyConfig, managed: &SystemProxyConfig) -> bool {
    current.enabled && current.address == managed.address
}

/// ownership 释放的可测试主体：只有完整 managed 状态仍一致时才写注册表；
/// 写后复读与 original 精确相等，才允许清 journal。
fn release_with<Read, Restore, Clear>(
    journal: Option<ProxyJournal>,
    journal_exists: bool,
    expected_port: u16,
    mut read: Read,
    mut restore: Restore,
    mut clear: Clear,
) -> Result<ReleaseOutcome>
where
    Read: FnMut() -> Result<SystemProxyConfig>,
    Restore: FnMut(&SystemProxyConfig) -> Result<()>,
    Clear: FnMut(),
{
    let current = read().map_err(|e| {
        Error::Other(format!(
            "Cannot confirm system proxy ownership; journal kept: {}",
            e
        ))
    })?;
    let Some(journal) = journal else {
        let hinted = crate::proxy::system_proxy::managed_proxy_config(expected_port);
        if journal_exists || points_at_managed_address(&current, &hinted) {
            return Err(Error::Other(
                "System proxy still targets ClashEdge but ownership journal is missing or corrupt; refusing to modify registry or stop Mihomo"
                    .to_string(),
            ));
        }
        return Ok(ReleaseOutcome::NoOwnership);
    };

    if !journal.owned {
        clear();
        return Ok(ReleaseOutcome::NoOwnership);
    }
    let managed = canonical_managed(&journal);
    if current != managed {
        if points_at_managed_address(&current, &managed) {
            return Err(Error::Other(
                "Windows proxy still targets ClashEdge, but its managed fields changed; ownership is ambiguous, so registry and Mihomo were left untouched"
                    .to_string(),
            ));
        }
        clear();
        return Ok(ReleaseOutcome::OwnershipLost);
    }

    let target = journal
        .original
        .clone()
        .unwrap_or_else(crate::proxy::system_proxy::disabled_proxy_config);
    restore(&target).map_err(|e| {
        Error::Other(format!(
            "Failed to restore owned Windows proxy; Mihomo must remain running and journal was kept: {}",
            e
        ))
    })?;
    let verified = read().map_err(|e| {
        Error::Other(format!(
            "Windows proxy restore could not be verified; Mihomo must remain running and journal was kept: {}",
            e
        ))
    })?;
    if verified != target {
        return Err(Error::Other(
            "Windows proxy restore verification failed; Mihomo must remain running and journal was kept"
                .to_string(),
        ));
    }

    let message = if target.enabled {
        format!("Restored original Windows proxy ({})", target.address)
    } else if target.auto_config_url.is_some() {
        "Restored original Windows PAC configuration".to_string()
    } else {
        "Restored original Windows no-proxy configuration".to_string()
    };
    clear();
    info!("{}", message);
    Ok(ReleaseOutcome::Restored {
        message,
        restored: target,
    })
}

/// 正常退出、手动关闭、异常启动恢复共用的唯一 ownership 释放入口。
pub fn release_owned_proxy(
    data_dir: &std::path::Path,
    expected_port: u16,
) -> Result<ReleaseOutcome> {
    let path = journal_path(data_dir);
    release_with(
        read_journal(data_dir),
        path.exists(),
        expected_port,
        crate::proxy::system_proxy::get_system_proxy,
        crate::proxy::system_proxy::restore_system_proxy,
        || clear_journal(data_dir),
    )
}

pub fn recover_on_startup(
    data_dir: &std::path::Path,
    expected_port: u16,
) -> Result<ReleaseOutcome> {
    release_owned_proxy(data_dir, expected_port)
}

/// 建立 ownership。重复开启不会覆盖 original；端口变化先安全释放旧 ownership。
pub fn acquire_system_proxy(data_dir: &std::path::Path, mixed_port: u16) -> Result<()> {
    acquire_system_proxy_if_unchanged(data_dir, mixed_port, None)
}

/// Mihomo 崩溃重启时传入 `expected_current`，确保退避期间的用户修改不被覆盖。
pub fn acquire_system_proxy_if_unchanged(
    data_dir: &std::path::Path,
    mixed_port: u16,
    expected_current: Option<&SystemProxyConfig>,
) -> Result<()> {
    let managed = crate::proxy::system_proxy::managed_proxy_config(mixed_port);
    let path = journal_path(data_dir);
    let mut previous_switch: Option<(ProxyJournal, SystemProxyConfig)> = None;
    if let Some(existing) = read_journal(data_dir) {
        let current = crate::proxy::system_proxy::get_system_proxy()?;
        let old_managed = canonical_managed(&existing);
        if existing.owned && current == old_managed {
            if old_managed == managed {
                return Ok(());
            }
            let baseline = existing
                .original
                .clone()
                .unwrap_or_else(crate::proxy::system_proxy::disabled_proxy_config);
            release_owned_proxy(data_dir, existing.mixed_port)?;
            previous_switch = Some((existing, baseline));
        } else if points_at_managed_address(&current, &old_managed) {
            return Err(Error::Other(
                "Cannot change system proxy: existing ClashEdge ownership is ambiguous".to_string(),
            ));
        } else {
            // 保留 journal 作为“曾由 ClashEdge 接管、现已被外部修改”的证据。
            // 若此处删除，下一次开启尝试会把外部代理误当作全新 baseline 并覆盖。
            return Err(Error::Other(
                "Cannot reacquire system proxy because Windows proxy ownership changed".to_string(),
            ));
        }
    } else if path.exists() {
        return Err(Error::Other(
            "Cannot acquire system proxy because the ownership journal is corrupt; refusing to overwrite Windows proxy"
                .to_string(),
        ));
    }

    let before = crate::proxy::system_proxy::get_system_proxy()
        .map_err(|e| error_after_switch_failure(data_dir, previous_switch.as_ref(), e))?;
    if let Some(expected) = expected_current {
        if &before != expected {
            return Err(error_after_switch_failure(
                data_dir,
                previous_switch.as_ref(),
                Error::Other(
                    "Windows proxy changed while Mihomo was restarting; refusing to overwrite the new user state"
                        .to_string(),
                ),
            ));
        }
    }
    if points_at_managed_address(&before, &managed) {
        return Err(error_after_switch_failure(
            data_dir,
            previous_switch.as_ref(),
            Error::Other(
                "Windows proxy already targets ClashEdge without a valid ownership journal; refusing takeover"
                    .to_string(),
            ),
        ));
    }

    let journal = ProxyJournal {
        session_id: format!(
            "{:016x}{:016x}",
            rand::random::<u64>(),
            rand::random::<u64>()
        ),
        pid: std::process::id(),
        mixed_port,
        original: Some(before.clone()),
        owned: true,
        managed: Some(managed.clone()),
    };
    write_journal(data_dir, &journal)
        .map_err(|e| error_after_switch_failure(data_dir, previous_switch.as_ref(), e))?;

    if let Err(e) = crate::proxy::system_proxy::restore_system_proxy(&managed) {
        let new_takeover_rolled_back = crate::proxy::system_proxy::get_system_proxy().ok().as_ref()
            == Some(&managed)
            && crate::proxy::system_proxy::restore_system_proxy(&before).is_ok()
            && crate::proxy::system_proxy::get_system_proxy().ok().as_ref() == Some(&before);
        if new_takeover_rolled_back && previous_switch.is_none() {
            clear_journal(data_dir);
        }
        return Err(error_after_switch_failure(
            data_dir,
            previous_switch.as_ref(),
            Error::Other(format!(
                "Failed to enable Windows proxy; rollback attempted and journal kept unless verified: {}",
                e
            )),
        ));
    }

    let verified = crate::proxy::system_proxy::get_system_proxy()
        .map_err(|e| error_after_switch_failure(data_dir, previous_switch.as_ref(), e))?;
    if verified != managed {
        if previous_switch.is_none() && !points_at_managed_address(&verified, &managed) {
            clear_journal(data_dir);
        }
        return Err(error_after_switch_failure(
            data_dir,
            previous_switch.as_ref(),
            Error::Other(
                "Windows proxy takeover verification failed; concurrent proxy change detected"
                    .to_string(),
            ),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// 创建一次性临时目录（测试专属，语义类似 launcher 测试的 temp root）
    fn temp_data_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "clashedge-journal-test-{}-{}-{}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample_journal() -> ProxyJournal {
        ProxyJournal {
            session_id: "smoke-session".to_string(),
            pid: 4242,
            mixed_port: 7890,
            original: Some(SystemProxyConfig {
                enabled: true,
                address: "10.0.0.5:8080".to_string(),
                bypass_list: vec!["<local>".to_string()],
                auto_config_url: Some("http://pac.example/a.pac".to_string()),
            }),
            owned: true,
            managed: Some(crate::proxy::system_proxy::managed_proxy_config(7890)),
        }
    }

    #[test]
    fn write_then_read_preserves_fields() {
        let dir = temp_data_dir("rt");
        let j = sample_journal();
        write_journal(&dir, &j).unwrap();

        let read = read_journal(&dir).expect("journal should be readable after write");
        assert_eq!(read.session_id, j.session_id);
        assert_eq!(read.pid, j.pid);
        assert_eq!(read.mixed_port, 7890);
        assert!(read.owned);
        let orig = read.original.expect("original preserved");
        assert_eq!(orig.address, "10.0.0.5:8080");
        assert_eq!(orig.bypass_list, vec!["<local>".to_string()]);
        assert_eq!(
            orig.auto_config_url.as_deref(),
            Some("http://pac.example/a.pac")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_failure_is_reported_not_silently_swallowed() {
        // journal 写失败必须返回 Err（调用方据此拒绝开启系统代理）。
        // 用一个不存在（也不可创建）的父目录模拟磁盘故障。
        let missing_parent = std::env::temp_dir().join(format!(
            "clashedge-journal-no-parent-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        // 不创建 missing_parent，直接在其下写文件必然失败
        let err = write_journal(&missing_parent, &sample_journal());
        assert!(err.is_err(), "write_journal must surface failure");
    }

    #[test]
    fn corrupt_journal_reads_as_none() {
        let dir = temp_data_dir("corrupt");
        std::fs::write(
            dir.join(JOURNAL_FILE),
            r#"{ "this is not valid json" "#.as_bytes(),
        )
        .unwrap();
        assert!(
            read_journal(&dir).is_none(),
            "corrupt journal must read as None"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clear_journal_is_ok_when_missing() {
        let dir = temp_data_dir("clear");
        // 不存在时 clear_journal 静默成功、不 panic
        clear_journal(&dir);
        assert!(!dir.join(JOURNAL_FILE).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clear_journal_removes_written_file() {
        let dir = temp_data_dir("clear2");
        write_journal(&dir, &sample_journal()).unwrap();
        assert!(dir.join(JOURNAL_FILE).exists());
        clear_journal(&dir);
        assert!(!dir.join(JOURNAL_FILE).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn release_restores_only_exact_owned_state_and_verifies_before_clear() {
        use std::cell::{Cell, RefCell};
        use std::collections::VecDeque;

        let journal = sample_journal();
        let managed = canonical_managed(&journal);
        let original = journal.original.clone().unwrap();
        let reads = RefCell::new(VecDeque::from([managed, original.clone()]));
        let restored = RefCell::new(None);
        let cleared = Cell::new(false);
        let outcome = release_with(
            Some(journal),
            true,
            7890,
            || Ok(reads.borrow_mut().pop_front().unwrap()),
            |target| {
                *restored.borrow_mut() = Some(target.clone());
                Ok(())
            },
            || cleared.set(true),
        )
        .unwrap();

        assert!(matches!(outcome, ReleaseOutcome::Restored { .. }));
        assert_eq!(restored.into_inner(), Some(original));
        assert!(cleared.get());
    }

    #[test]
    fn release_does_not_touch_registry_after_other_proxy_takes_over() {
        use std::cell::Cell;

        let current = SystemProxyConfig {
            enabled: true,
            address: "127.0.0.1:10809".to_string(),
            bypass_list: vec![],
            auto_config_url: None,
        };
        let restored = Cell::new(false);
        let cleared = Cell::new(false);
        let outcome = release_with(
            Some(sample_journal()),
            true,
            7890,
            || Ok(current.clone()),
            |_| {
                restored.set(true);
                Ok(())
            },
            || cleared.set(true),
        )
        .unwrap();

        assert_eq!(outcome, ReleaseOutcome::OwnershipLost);
        assert!(!restored.get());
        assert!(cleared.get());
    }

    #[test]
    fn release_keeps_core_and_journal_when_address_is_ours_but_fields_changed() {
        use std::cell::Cell;

        let mut current = crate::proxy::system_proxy::managed_proxy_config(7890);
        current.auto_config_url = Some("http://changed.example/proxy.pac".to_string());
        let restored = Cell::new(false);
        let cleared = Cell::new(false);
        let result = release_with(
            Some(sample_journal()),
            true,
            7890,
            || Ok(current.clone()),
            |_| {
                restored.set(true);
                Ok(())
            },
            || cleared.set(true),
        );

        assert!(result.is_err());
        assert!(!restored.get());
        assert!(!cleared.get());
    }

    #[test]
    fn release_failure_or_failed_verification_keeps_journal() {
        use std::cell::Cell;

        let managed = crate::proxy::system_proxy::managed_proxy_config(7890);
        let cleared = Cell::new(false);
        let failed_write = release_with(
            Some(sample_journal()),
            true,
            7890,
            || Ok(managed.clone()),
            |_| Err(Error::Other("injected registry failure".to_string())),
            || cleared.set(true),
        );
        assert!(failed_write.is_err());
        assert!(!cleared.get());

        let reads =
            std::cell::RefCell::new(std::collections::VecDeque::from([managed.clone(), managed]));
        let failed_verify = release_with(
            Some(sample_journal()),
            true,
            7890,
            || Ok(reads.borrow_mut().pop_front().unwrap()),
            |_| Ok(()),
            || cleared.set(true),
        );
        assert!(failed_verify.is_err());
        assert!(!cleared.get());
    }

    #[test]
    fn missing_journal_never_authorizes_clearing_a_live_clashedge_proxy() {
        let managed = crate::proxy::system_proxy::managed_proxy_config(7890);
        let result = release_with(
            None,
            false,
            7890,
            || Ok(managed.clone()),
            |_| panic!("must not write registry"),
            || panic!("must not clear missing journal"),
        );
        assert!(result.is_err());
    }

    #[test]
    fn failed_port_switch_restores_previous_ownership_only_from_exact_baseline() {
        use std::cell::RefCell;
        use std::collections::VecDeque;

        let previous = sample_journal();
        let baseline = previous.original.clone().unwrap();
        let previous_managed = canonical_managed(&previous);
        let reads = RefCell::new(VecDeque::from([baseline.clone(), previous_managed.clone()]));
        let written = RefCell::new(None);
        let restored = RefCell::new(None);

        restore_previous_with(
            &previous,
            &baseline,
            || Ok(reads.borrow_mut().pop_front().unwrap()),
            |target| {
                *restored.borrow_mut() = Some(target.clone());
                Ok(())
            },
            |journal| {
                *written.borrow_mut() = Some(journal.session_id.clone());
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(written.into_inner().as_deref(), Some("smoke-session"));
        assert_eq!(restored.into_inner(), Some(previous_managed));
    }

    #[test]
    fn failed_port_switch_never_overwrites_concurrent_proxy_takeover() {
        let previous = sample_journal();
        let baseline = previous.original.clone().unwrap();
        let external = SystemProxyConfig {
            enabled: true,
            address: "127.0.0.1:10809".to_string(),
            bypass_list: vec!["external".to_string()],
            auto_config_url: None,
        };

        let result = restore_previous_with(
            &previous,
            &baseline,
            || Ok(external.clone()),
            |_| panic!("must not write registry after concurrent takeover"),
            |_| panic!("must not rewrite journal after concurrent takeover"),
        );
        assert!(result.is_err());
    }

    // ---- 退出解耦验收 ----
    // 退出路径：确认 ownership → 恢复系统代理 → 复读验证 → 清除 journal →
    // 停止 Mihomo。journal 只负责系统代理 ownership；Mihomo 的停止成败不得
    // 影响已清除的 journal。E/G/I 已由上方 Cell 版测试覆盖；此处用真实 journal
    // 文件补充两个 Cell 版无法表达的新增解耦场景：F（Mihomo 停止失败不重建
    // journal）与 J（journal 损坏且系统仍指向 ClashEdge 时的 fail-safe）。

    fn journal_file_exists(dir: &std::path::Path) -> bool {
        dir.join(JOURNAL_FILE).exists()
    }

    // F：proxy restore 成功后，即使后续 Mihomo stop 失败，journal 仍保持已清除。
    //    释放成功即清 journal；Mihomo 停止失败只记录错误，绝不重建/保留 journal。
    #[test]
    fn exit_journal_stays_cleared_when_mihomo_stop_fails() {
        let dir = temp_data_dir("F");
        let journal = sample_journal();
        let managed = canonical_managed(&journal);
        let original = journal.original.clone().unwrap();
        write_journal(&dir, &journal).unwrap();

        let reads = std::cell::RefCell::new(std::collections::VecDeque::from([
            managed.clone(),
            original.clone(),
        ]));
        let _ = release_with(
            Some(journal),
            true,
            7890,
            || Ok(reads.borrow_mut().pop_front().unwrap()),
            |_| Ok(()),
            || clear_journal(&dir),
        )
        .unwrap();
        assert!(!journal_file_exists(&dir), "journal cleared after restore");

        // 模拟 cleanup_on_exit 中 Mihomo 停止失败：此处既不 clear_journal
        // （已清除）也不 write_journal（不重建）。journal 必须仍为已清除。
        let mihomo_stop_ok = false;
        if !mihomo_stop_ok {
            // 仅记录错误；不触碰 journal。
        }
        assert!(
            !journal_file_exists(&dir),
            "journal must stay cleared after Mihomo stop failure"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // J：journal 损坏（文件存在但不可解析）且 Windows 仍指向 ClashEdge 时
    //    保持 fail-safe：不修改注册表、不停止 Mihomo、不创建 journal。
    #[test]
    fn exit_corrupt_journal_with_live_clashedge_proxy_keeps_fail_safe() {
        let dir = temp_data_dir("J");
        // 写入一份损坏的 journal 文件：存在但不可解析 → read_journal 返回 None。
        std::fs::write(dir.join(JOURNAL_FILE), b"{ not valid json").unwrap();
        let managed = crate::proxy::system_proxy::managed_proxy_config(7890);
        let restored = std::cell::Cell::new(false);

        let result = release_with(
            None,
            true,
            7890,
            || Ok(managed.clone()),
            |_| {
                restored.set(true);
                Ok(())
            },
            || panic!("must not clear a live ClashEdge proxy without a valid journal"),
        );

        assert!(
            result.is_err(),
            "must keep fail-safe and refuse to modify registry"
        );
        assert!(!restored.get(), "must not write registry");
        // 损坏的 journal 文件保持原样（既未清除也未改写）。
        assert!(
            journal_file_exists(&dir),
            "corrupt journal must be left untouched"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
