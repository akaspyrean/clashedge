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

/// 写入 journal（原子写）。失败只告警不阻断主流程——journal 是尽力而为
/// 的恢复机制，写不进去不应阻止系统代理开启。
pub fn write_journal(data_dir: &std::path::Path, journal: &ProxyJournal) {
    match serde_json::to_string_pretty(journal) {
        Ok(json) => {
            if let Err(e) = atomic_write(&journal_path(data_dir), json.as_bytes()) {
                warn!("Failed to write proxy session journal: {}", e);
            } else {
                info!(
                    "Proxy session journal written (session {})",
                    journal.session_id
                );
            }
        }
        Err(e) => warn!("Failed to serialize proxy session journal: {}", e),
    }
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
pub fn recover_on_startup(data_dir: &std::path::Path) -> Option<String> {
    let journal = read_journal(data_dir)?;
    // 以 journal 记录的端口为准（更贴近"当时"的状态）
    let ours = format!("127.0.0.1:{}", journal.mixed_port);

    let current = crate::proxy::system_proxy::get_system_proxy().ok();
    let stale = matches!(&current, Some(c) if c.enabled && c.address == ours);

    // 无论是否命中恢复条件，残留 journal 都已失去意义，读取后即清理
    clear_journal(data_dir);

    if !stale {
        return None;
    }

    match &journal.original {
        Some(orig) if orig.enabled => {
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
                    Some(msg)
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
    }
}
