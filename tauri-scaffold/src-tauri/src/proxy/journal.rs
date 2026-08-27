// src-tauri/src/proxy/journal.rs
//! P1-8 系统代理 Recovery Journal（proxy-session.json）
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
}

fn journal_path(data_dir: &std::path::Path) -> PathBuf {
    data_dir.join(JOURNAL_FILE)
}

/// 写入 journal（原子写）。
///
/// P0-1：journal 是崩溃恢复的唯一凭据——必须先于"改注册表"持久化成功，
/// 否则进程在改完注册表后崩溃、journal 不在，下次启动无法恢复用户原代理。
/// 因此本函数失败必须返回 Err，由调用方决定是否拒绝开启系统代理。
/// （旧行为只 `warn!` 不阻断 → 注册表已改但 journal 缺失 → 崩溃后死代理。）
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

/// 启动自愈：检测上次会话是否异常结束且系统代理仍被本应用占用。
///
/// 返回 `Some(恢复说明)` 表示执行了一次恢复（已把注册表代理改回原始状态）。
/// 判定条件（全部满足才动作，避免误伤用户手动设置的代理）：
/// 1. journal 存在（上次会话开启过系统代理且未走干净关闭路径）；
/// 2. 当前注册表代理 enabled 且 address == `127.0.0.1:{mixed_port}`
///    （仍指向本应用端口 = 上次进程没来得及还原）。
///
/// P0-1：恢复成功后才删 journal。旧行为是"读完即删"，恢复失败的话 journal
/// 已不在，下次启动再无凭据继续恢复。新顺序：
/// 1. 读取 journal + 当前注册表状态；
/// 2. 若不命中恢复条件（无 journal / 非指向本应用的死代理）→ 删 journal 收尾；
/// 3. 若命中 → 执行恢复；**恢复成功才删 journal**；恢复失败时保留 journal，
///    下次启动可继续尝试恢复。
pub fn recover_on_startup(data_dir: &std::path::Path) -> Option<String> {
    let journal = read_journal(data_dir)?;
    // 以 journal 记录的端口为准（更贴近"当时"的状态）
    let ours = format!("127.0.0.1:{}", journal.mixed_port);

    // 判定当前注册表代理是否仍指向本应用的死代理。
    // 区分「读取成功但非本应用接管」与「注册表读取失败」：
    // - 读取成功且非本应用接管（用户已手动改回 / 其他工具接管）→ journal 失去
    //   意义，清理收尾；
    // - **注册表读取失败** → 无法判定当前状态，是临时的还是永久未知。
    //   此时**必须保留 journal**（不清），返回 None 待下次启动再试——否则先
    //   删凭据再判定，正是审计 P0-1 指出的"先删后恢复"边界。
    let current = match crate::proxy::system_proxy::get_system_proxy() {
        Ok(c) => c,
        Err(e) => {
            warn!(
                "Cannot read system proxy registry state on startup (keeping journal for retry): {}",
                e
            );
            // P0-1：注册表读取失败 → 无法判定当前状态，保留 journal 下次启动再试。
            // 绝不能在这里 clear_journal——否则异常退出残留的死代理会永久失去恢复凭据。
            return None;
        }
    };
    let stale = current.enabled && current.address == ours;

    if !stale {
        // journal 残留但当前注册表不再指向本应用的死代理（用户已手动改回 / 已被
        // 其他工具接管）→ journal 已无恢复意义，清理收尾。
        clear_journal(data_dir);
        return None;
    }

    // 命中恢复条件：当前是死代理，必须还原到 journal.original 记录的用户原状态。
    let restore_outcome = match &journal.original {
        Some(orig) if orig.enabled => {
            // 用户原本有静态代理：还原为原 address / bypass / PAC
            match crate::proxy::system_proxy::set_system_proxy(
                true,
                &orig.address,
                &orig.bypass_list,
                orig.auto_config_url.as_deref(),
            ) {
                Ok(()) => {
                    let msg = format!(
                        "Recovered system proxy to original ({}) after abnormal exit",
                        orig.address
                    );
                    info!("{}", msg);
                    Some(msg)
                }
                Err(e) => {
                    let msg = format!(
                        "Stale proxy pointed at dead {} but restore failed: {}",
                        ours, e
                    );
                    warn!("{}", msg);
                    // 恢复失败至少要把死代理关掉，不能留着断网；
                    // 用户原有的 PAC（若快照有）一并写回
                    let _ = crate::proxy::system_proxy::set_system_proxy(
                        false,
                        "",
                        &[],
                        journal
                            .original
                            .as_ref()
                            .and_then(|o| o.auto_config_url.as_deref()),
                    );
                    None
                }
            }
        }
        _ => {
            // 原本未启用静态代理：清掉残留的死代理；若用户原有 PAC 则写回还原
            let pac = journal
                .original
                .as_ref()
                .and_then(|o| o.auto_config_url.as_deref());
            match crate::proxy::system_proxy::set_system_proxy(false, "", &[], pac) {
                Ok(()) => {
                    let msg = format!(
                        "Cleared stale proxy pointing at dead {} after abnormal exit",
                        ours
                    );
                    info!("{}", msg);
                    Some(msg)
                }
                Err(e) => {
                    warn!("Failed to clear stale proxy after abnormal exit: {}", e);
                    None
                }
            }
        }
    };

    // P0-1：恢复成功才删 journal；恢复失败保留 journal，下次启动可再试。
    if restore_outcome.is_some() {
        clear_journal(data_dir);
    }
    restore_outcome
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
        // P0-1 语义：journal 写失败必须返回 Err（调用方据此拒绝开启系统代理）。
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
}
