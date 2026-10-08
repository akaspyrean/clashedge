// src-tauri/src/core/spawn.rs
//! mihomo 进程的"准备 + 启动"共用实现（审计 A1 / B4 / B5）。
//!
//! 首次启动（`lifecycle::start_locked`）与崩溃自动重启（`supervisor` watcher）
//! 以前各自手写了一份"读 profile → 写 runtime → 建日志 → spawn"，两处行为已漂移
//! （自动重启不轮换日志、不检测端口冲突、`File::create` 截断了崩溃现场日志）。
//! 现在统一走本模块：
//! - [`write_runtime_config`]：生成并原子写入 runtime-config.yaml，返回内容哈希；
//! - [`spawn_mihomo`]：日志**追加**写入（启动时按大小轮转）+ `CREATE_NO_WINDOW` +
//!   落盘 `core-session.json`（pid + 映像路径），供下次启动回收孤儿进程；
//! - [`preflight`]：spawn 前预检 mixed-port / DNS / 控制器端口，必要时回收
//!   本应用遗留的孤儿 mihomo，并给出"谁占用了端口"的可操作错误。

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::process::{Child, Command};
use tracing::{info, warn};

use crate::config::model::Config;
use crate::core::config::build_runtime_config;
use crate::util::error::{Error, Result};
use crate::util::paths::sanitize_profile_name;
use crate::util::process::{owns_pid, port_owner, terminate_owned};

/// 单个 mihomo 日志文件的轮转阈值
const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

pub fn runtime_config_path(data_dir: &Path) -> PathBuf {
    data_dir.join("runtime-config.yaml")
}

pub fn stdout_log_path(data_dir: &Path) -> PathBuf {
    data_dir.join("logs").join("mihomo-stdout.log")
}

// ---------------------------------------------------------------------------
// runtime-config
// ---------------------------------------------------------------------------

/// 读取激活 Profile 的原始内容（名称经净化，防路径穿越）。
/// 内置预设 `DIRECT` 没有对应文件，直接返回 None（不再每次启动打 warn）。
pub fn read_active_profile(data_dir: &Path, config: &Config) -> Option<String> {
    let name = config.general.profile.trim();
    if name.is_empty() {
        return None;
    }
    let safe = sanitize_profile_name(name).ok()?;
    let path = data_dir.join("profiles").join(format!("{}.yaml", safe));
    match std::fs::read_to_string(&path) {
        Ok(content) => Some(content),
        Err(_) => {
            if !safe.eq_ignore_ascii_case("DIRECT") {
                warn!(
                    "Active profile '{}' not found at {:?}; using builtin preset",
                    safe, path
                );
            }
            None
        }
    }
}

/// 生成并原子写入运行时配置，返回 `(路径, 内容 sha256 前 16 位)`。
/// 哈希用于"配置无实质变化则跳过热重载"（审计 B8）。
pub fn write_runtime_config(data_dir: &Path, config: &Config) -> Result<(PathBuf, String)> {
    let profile = read_active_profile(data_dir, config);
    let runtime = build_runtime_config(config, profile.as_deref())?;
    let yaml = serde_yaml::to_string(&runtime)?;
    let path = runtime_config_path(data_dir);
    crate::util::atomic::atomic_write(&path, yaml.as_bytes())?;
    let digest = Sha256::digest(yaml.as_bytes());
    let hash: String = digest
        .iter()
        .take(8)
        .map(|b| format!("{:02x}", b))
        .collect();
    info!("Runtime config written to {:?} (hash {})", path, hash);
    Ok((path, hash))
}

// ---------------------------------------------------------------------------
// 内置规则集文件兜底
// ---------------------------------------------------------------------------

/// 内置 rule-provider 是本地 `type: file`：文件缺失会让 mihomo 拒绝启动。
/// 启动前补齐缺失的内置规则文件——优先从随包的 `App/DefaultData/rules` 复制，
/// 否则写入一个只含占位规则的最小文件（保证内核能起来，签名更新器稍后补全）。
pub fn ensure_rule_files(data_dir: &Path) {
    let rules_dir = data_dir.join("rules");
    if std::fs::create_dir_all(&rules_dir).is_err() {
        return;
    }
    let defaults = crate::util::paths::portable_root()
        .map(|r| r.join("App").join("DefaultData").join("rules"));
    for name in crate::config::model::BUILTIN_RULE_SETS {
        let file = rules_dir.join(format!("{}.yaml", name));
        if file.exists() {
            continue;
        }
        let copied = defaults
            .as_ref()
            .map(|d| d.join(format!("{}.yaml", name)))
            .filter(|src| src.exists())
            .map(|src| std::fs::copy(src, &file).is_ok())
            .unwrap_or(false);
        if !copied {
            warn!(
                "Builtin rule set '{}' missing; writing a minimal placeholder",
                name
            );
            let _ = std::fs::write(
                &file,
                "payload:
  - DOMAIN,clashedge-placeholder.invalid
",
            );
        }
    }
}

// ---------------------------------------------------------------------------
// core-session.json（孤儿回收凭据）
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
struct CoreSession {
    pid: u32,
    image_path: String,
    started_at: u64,
}

fn session_path(data_dir: &Path) -> PathBuf {
    data_dir.join("core-session.json")
}

fn record_session(data_dir: &Path, pid: u32, image: &Path) {
    let s = CoreSession {
        pid,
        image_path: image.to_string_lossy().to_string(),
        started_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    };
    match serde_json::to_vec_pretty(&s) {
        Ok(bytes) => {
            if let Err(e) = crate::util::atomic::atomic_write(&session_path(data_dir), &bytes) {
                warn!("Failed to record core session: {}", e);
            }
        }
        Err(e) => warn!("Failed to serialize core session: {}", e),
    }
}

/// 子进程确认退出后清除会话记录。
pub fn clear_session(data_dir: &Path) {
    let _ = std::fs::remove_file(session_path(data_dir));
}

/// 回收上次会话遗留的孤儿 mihomo（应用被强杀/崩溃后内核仍在运行）。
/// 只终止"PID 仍存在且映像路径确认为本应用 mihomo"的进程；其余情况只清记录。
/// 返回是否终止了一个孤儿。
pub fn reclaim_orphan(data_dir: &Path, mihomo_path: &Path) -> Result<bool> {
    let path = session_path(data_dir);
    let Ok(content) = std::fs::read_to_string(&path) else {
        return Ok(false);
    };
    let Ok(session) = serde_json::from_str::<CoreSession>(&content) else {
        let _ = std::fs::remove_file(&path);
        return Ok(false);
    };
    let recorded = PathBuf::from(&session.image_path);
    // 记录的映像路径、当前期望的 mihomo 路径都必须与进程实际映像一致。
    let ours = owns_pid(session.pid, Some(&recorded)) && owns_pid(session.pid, Some(mihomo_path));
    if !ours {
        let _ = std::fs::remove_file(&path);
        return Ok(false);
    }
    warn!(
        "Found orphaned mihomo from a previous session (PID {}); terminating",
        session.pid
    );
    match terminate_owned(session.pid, mihomo_path, 3000) {
        Ok(true) => {
            let _ = std::fs::remove_file(&path);
            Ok(true)
        }
        Ok(false) => {
            // 进程已消失（竞态）或未在期限内退出——清记录；端口预检会兜底判断。
            let _ = std::fs::remove_file(&path);
            Ok(false)
        }
        Err(e) => Err(e),
    }
}

// ---------------------------------------------------------------------------
// 日志
// ---------------------------------------------------------------------------

/// 超过阈值则轮转为 `.old.log`（只保留一份上一会话）。
fn rotate_if_large(path: &Path) {
    if let Ok(meta) = std::fs::metadata(path) {
        if meta.len() > MAX_LOG_BYTES {
            let _ = std::fs::remove_file(path.with_extension("old.log"));
            let _ = std::fs::rename(path, path.with_extension("old.log"));
        }
    }
}

/// 追加方式打开日志并写一行会话分隔，返回 `(文件, 打开前长度)`。
/// 追加而非 `File::create`：崩溃循环里不会覆盖第一次崩溃的现场。
fn open_log_append(path: &Path, label: &str) -> Result<(std::fs::File, u64)> {
    use std::io::Write;
    rotate_if_large(path);
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| {
            Error::Io(std::io::Error::other(format!(
                "open {}: {}",
                path.display(),
                e
            )))
        })?;
    let offset = file.metadata().map(|m| m.len()).unwrap_or(0);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let _ = writeln!(
        file,
        "# ===== {} session start (unix {}) =====",
        label, stamp
    );
    let offset_after_marker = file.metadata().map(|m| m.len()).unwrap_or(offset);
    Ok((file, offset_after_marker))
}

/// spawn 结果：子进程 + 本次会话在 stdout 日志中的起始偏移
/// （端口冲突检测只读本会话新增的内容，避免历史会话的 bind 错误造成误报）。
pub struct Spawned {
    pub child: Child,
    pub stdout_offset: u64,
}

/// 启动 mihomo：`-d <data_dir> -f <runtime_config>`，stdout/stderr 追加到日志文件。
pub fn spawn_mihomo(mihomo_path: &Path, data_dir: &Path, runtime_config: &Path) -> Result<Spawned> {
    let logs_dir = data_dir.join("logs");
    let _ = std::fs::create_dir_all(&logs_dir);
    let (stdout_file, stdout_offset) =
        open_log_append(&logs_dir.join("mihomo-stdout.log"), "mihomo stdout")?;
    let (stderr_file, _) = open_log_append(&logs_dir.join("mihomo-stderr.log"), "mihomo stderr")?;

    let mut cmd = Command::new(mihomo_path);
    cmd.arg("-d").arg(data_dir).arg("-f").arg(runtime_config);
    cmd.current_dir(data_dir);
    cmd.stdout(std::process::Stdio::from(stdout_file))
        .stderr(std::process::Stdio::from(stderr_file));
    #[cfg(target_os = "windows")]
    {
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let child = cmd.spawn().map_err(Error::from)?;
    if let Some(pid) = child.id() {
        record_session(data_dir, pid, mihomo_path);
    }
    Ok(Spawned {
        child,
        stdout_offset,
    })
}

/// 读取 stdout 日志中 `offset` 之后新增的内容。
pub fn read_log_since(data_dir: &Path, offset: u64) -> String {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(stdout_log_path(data_dir)) else {
        return String::new();
    };
    if f.seek(SeekFrom::Start(offset)).is_err() {
        return String::new();
    }
    let mut buf = Vec::new();
    let _ = f.read_to_end(&mut buf);
    String::from_utf8_lossy(&buf).to_string()
}

// ---------------------------------------------------------------------------
// preflight：端口预检
// ---------------------------------------------------------------------------

/// 预检结果
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Preflight {
    /// 配置的控制器端口被占用时改选的空闲地址（仅内存生效，见 lifecycle）
    pub controller_override: Option<String>,
}

fn port_of(addr: &str) -> Option<u16> {
    addr.trim().rsplit(':').next()?.parse().ok()
}

fn try_bind(host: &str, port: u16) -> std::io::Result<()> {
    std::net::TcpListener::bind((host, port)).map(|_| ())
}

/// 确保 `host:port` 可绑定。被本应用遗留的 mihomo 占用时回收它后重试；
/// 被其他进程占用时返回带进程名与 PID 的错误。
fn ensure_bindable(label: &str, host: &str, port: u16, mihomo_path: &Path) -> Result<()> {
    if try_bind(host, port).is_ok() {
        return Ok(());
    }
    if let Some((pid, name)) = port_owner(port) {
        if owns_pid(pid, Some(mihomo_path)) {
            warn!(
                "{} port {} is held by a leftover ClashEdge mihomo (PID {}); reclaiming",
                label, port, pid
            );
            terminate_owned(pid, mihomo_path, 3000)?;
            for _ in 0..10 {
                if try_bind(host, port).is_ok() {
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(200));
            }
        }
        return Err(Error::Other(format!(
            "端口 {}（{}）已被进程 {}（PID {}）占用。请关闭该程序，或在设置中更换端口后重试。",
            port, label, name, pid
        )));
    }
    Err(Error::Other(format!(
        "端口 {}（{}）无法绑定，且未找到占用进程。该端口可能位于 Windows 保留端口范围\
         （netsh int ipv4 show excludedportrange protocol=tcp），请更换端口后重试。",
        port, label
    )))
}

fn free_local_port() -> std::io::Result<u16> {
    Ok(std::net::TcpListener::bind(("127.0.0.1", 0))?
        .local_addr()?
        .port())
}

/// spawn 前预检（同步，调用方应放在 `spawn_blocking` 中）：
/// - mixed-port：必须可绑定，否则报错；
/// - DNS listen（启用时）：必须可绑定，否则报错；
/// - 控制器端口：被占用时自动改选空闲端口，避免与其他 Clash 系软件的
///   9090 冲突、更避免误连到别人的控制器。
pub fn preflight(mihomo_path: &Path, cfg: &Config) -> Result<Preflight> {
    let mut out = Preflight::default();

    let mixed_host = if cfg.general.allow_lan {
        "0.0.0.0"
    } else {
        "127.0.0.1"
    };
    ensure_bindable(
        "mixed-port",
        mixed_host,
        cfg.general.mixed_port,
        mihomo_path,
    )?;

    if cfg.dns.enable {
        let listen = crate::core::health::normalize_dns_listen(&cfg.dns.listen);
        if let Some(port) = port_of(&listen) {
            let host = listen
                .rsplit_once(':')
                .map(|(h, _)| h)
                .unwrap_or("127.0.0.1");
            let host = host.trim_matches(|c| c == '[' || c == ']');
            let host = if host.is_empty() { "127.0.0.1" } else { host };
            ensure_bindable("dns.listen", host, port, mihomo_path)?;
        }
    }

    if let Some(port) = port_of(&cfg.proxy.external_controller) {
        if ensure_bindable("external-controller", "127.0.0.1", port, mihomo_path).is_err() {
            let free = free_local_port()
                .map_err(|e| Error::Other(format!("no free port for controller: {}", e)))?;
            let addr = format!("127.0.0.1:{}", free);
            warn!(
                "external-controller port {} is occupied; using {} for this session",
                port, addr
            );
            out.controller_override = Some(addr);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("clashedge-spawn-{}-{}", tag, std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn ensure_rule_files_creates_missing_placeholders_without_overwriting() {
        let d = scratch("rules");
        std::fs::create_dir_all(d.join("rules")).unwrap();
        std::fs::write(
            d.join("rules").join("ai.yaml"),
            "payload:
  - DOMAIN,keep.me
",
        )
        .unwrap();
        ensure_rule_files(&d);
        assert!(std::fs::read_to_string(d.join("rules").join("ai.yaml"))
            .unwrap()
            .contains("keep.me"));
        for n in crate::config::model::BUILTIN_RULE_SETS {
            assert!(
                d.join("rules").join(format!("{}.yaml", n)).exists(),
                "{}",
                n
            );
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn port_of_parses_host_port() {
        assert_eq!(port_of("127.0.0.1:9090"), Some(9090));
        assert_eq!(port_of("[::1]:9090"), Some(9090));
        assert_eq!(port_of("nonsense"), None);
    }

    #[test]
    fn log_is_appended_not_truncated_and_offset_skips_history() {
        let d = scratch("log");
        let p = d.join("mihomo-stdout.log");
        std::fs::write(&p, "level=error msg=\"old bind failure\"\n").unwrap();
        let (_f, offset) = open_log_append(&p, "test").unwrap();
        let content = std::fs::read_to_string(&p).unwrap();
        assert!(content.contains("old bind failure"), "history preserved");
        assert!(offset > 0);
        // 偏移之后没有历史内容
        let tail = &content.as_bytes()[offset as usize..];
        assert!(tail.is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn rotation_moves_large_log_aside() {
        let d = scratch("rot");
        let p = d.join("mihomo-stdout.log");
        std::fs::write(&p, vec![b'a'; (MAX_LOG_BYTES + 1) as usize]).unwrap();
        rotate_if_large(&p);
        assert!(!p.exists());
        assert!(d.join("mihomo-stdout.old.log").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn reclaim_orphan_never_kills_unrelated_process() {
        let d = scratch("orphan");
        // 记录指向当前测试进程，但声称映像是别的文件 → 不是 mihomo，不得终止。
        let s = CoreSession {
            pid: std::process::id(),
            image_path: r"C:\fake\clash-edge-core.exe".into(),
            started_at: 0,
        };
        std::fs::write(session_path(&d), serde_json::to_vec(&s).unwrap()).unwrap();
        let r = reclaim_orphan(&d, Path::new(r"C:\fake\clash-edge-core.exe")).unwrap();
        assert!(!r);
        assert!(!session_path(&d).exists(), "stale record is cleared");
        // 损坏记录 / 缺失记录均安全
        std::fs::write(session_path(&d), b"{not json").unwrap();
        assert!(!reclaim_orphan(&d, Path::new(r"C:\x.exe")).unwrap());
        assert!(!reclaim_orphan(&d, Path::new(r"C:\x.exe")).unwrap());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn preflight_reports_owner_of_occupied_mixed_port() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let mut cfg = Config::default();
        cfg.general.mixed_port = port;
        let err = preflight(Path::new(r"C:\not\mihomo.exe"), &cfg).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains(&port.to_string()), "{}", msg);
        assert!(msg.contains(&std::process::id().to_string()), "{}", msg);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn preflight_picks_new_controller_port_when_occupied() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let busy = l.local_addr().unwrap().port();
        let mut cfg = Config::default();
        // mixed / dns 选两个空闲端口，避免与本机服务冲突
        cfg.general.mixed_port = free_local_port().unwrap();
        cfg.dns.enable = false;
        cfg.proxy.external_controller = format!("127.0.0.1:{}", busy);
        let pre = preflight(Path::new(r"C:\not\mihomo.exe"), &cfg).unwrap();
        let ov = pre.controller_override.expect("must override");
        assert_ne!(port_of(&ov), Some(busy));
    }
}
