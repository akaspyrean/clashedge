// src-tauri/src/util/paths.rs
//! 路径工具：处理数据目录、mihomo 二进制路径
//!
//! 便携模式检测（自愈）：
//!   1. 便携标记：`<exe_dir>/App/portable.dat` 存在
//!   2. 便携实证：`<exe_dir>/App/clash-edge-core.exe` 存在（不依赖 portable.dat，
//!      复制/改名/换盘符后 portable.dat 可能丢失，但内核在 App/ 即说明便携）
//!   3. `App/` 目录存在即判定便携（即使 portable.dat 与内核都缺失，如部分解压/
//!      杀软拦截内核/新旧包文件混用——此时报「App/clash-edge-core.exe 缺失」的
//!      明确提示，而不是误判安装版去查 sidecar 目录）
//!   4. Tauri 默认 app_data_dir()
//!
//! 便携模式下，mihomo **始终** 位于 `<exe_dir>/App/clash-edge-core.exe`，
//! 数据目录固定 `<exe_dir>/Data/`，不回退 %APPDATA%，避免打包后找不到内核。

use crate::util::error::{Error, Result};
use std::path::{Path, PathBuf};
use tauri::Manager;

/// 规范化 exe 路径：去掉 Windows 的 `\\?\` / `\??\` / `\\.\` 前缀。
///
/// `std::env::current_exe()` 在 Windows 上可能返回带 `\\?\` 前缀的
/// verbatim 路径（如 `\\?\C:\Portable Files\ClashEdge\ClashEdge.exe`）。
/// 该前缀会令后续 `join("App").exists()` 等检查在部分路径布局下失败，
/// 导致便携模式误判为安装版（用户实测踩中：报 sidecar 缺失）。
/// 去掉前缀后返回普通 DOS 路径，`exists()` / `is_dir()` 行为恢复正常。
fn strip_verbatim_prefix(path: PathBuf) -> PathBuf {
    let s = path.to_string_lossy();
    let trimmed = if let Some(rest) = s.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else if let Some(rest) = s.strip_prefix(r"\??\") {
        PathBuf::from(rest)
    } else if let Some(rest) = s.strip_prefix(r"\\.\") {
        PathBuf::from(rest)
    } else {
        path
    };
    trimmed
}

/// 参考包 C# 启动器布局：数据目录由环境变量 `CLASH_EDGE_DATA_DIR` 显式指定。
///
/// 0.8.5 参考包（`docs/ClashEdge-portable-0.8.5.zip`）的根 `ClashEdge.exe` 是一个
/// C# 启动器：它把 `CLASH_EDGE_DATA_DIR` 指向包根 `Data/`，同时把 `HOME`/
/// `APPDATA`/`LOCALAPPDATA` 也重定向进 `Data/`，再启动 `App\ClashEdge\ClashEdge.exe`
/// （Tauri 应用本体）。因此内层 exe 不能靠 `current_exe` 旁边的 `portable.dat`
/// 判断数据目录（它在 `App\ClashEdge\` 子目录里），必须优先读环境变量。
///
/// 当前源码同时支持这套布局与原生便携布局（`App/portable.dat`），二者不互斥。
fn env_data_dir() -> Option<PathBuf> {
    std::env::var_os("CLASH_EDGE_DATA_DIR")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
}

/// 获取规范化后的当前 exe 路径（去掉 `\\?\` 前缀）。
fn current_exe_normalized() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .map(strip_verbatim_prefix)
}

/// 是否为便携模式：
/// - `CLASH_EDGE_DATA_DIR` 环境变量已设置（参考包 C# 启动器布局）→ 便携；
/// - **或** exe 同目录下 `App/portable.dat` 存在，或 `App/clash-edge-core.exe` 存在
///   （原生便携布局，复制/改名后 portable.dat 丢失时仍判定）。
pub fn is_portable_mode() -> bool {
    env_data_dir().is_some() || portable_indicators(current_exe_normalized().as_deref()).0
}

/// 便携根目录（包根，即 Data/ 的父目录）：
/// - 参考包布局：`CLASH_EDGE_DATA_DIR` 指向 `<root>/Data`，取其父目录；
/// - 原生便携布局：exe 所在目录。
pub fn portable_root() -> Option<PathBuf> {
    if let Some(data) = env_data_dir() {
        return data.parent().map(|p| p.to_path_buf());
    }
    current_exe_normalized()
        .and_then(|exe| exe.parent().map(|p| p.to_path_buf()))
}

/// 便携判定依据。
///
/// 返回 `(is_portable, app_dir, portable_marker_existed)`。
/// 作为纯函数被测试；生产调用 `portable_indicators(current_exe_dir)`。
pub fn portable_indicators(exe_dir: Option<&Path>) -> (bool, Option<PathBuf>, bool) {
    let Some(root) = exe_dir else {
        return (false, None, false);
    };
    let app_dir = root.join("App");

    // 先看便携标记文件
    let marker = app_dir.join("portable.dat");
    if marker.exists() {
        return (true, Some(app_dir), true);
    }

    // 标记丢失但内核就在 App/ 里 → 仍视为便携（复制/改名/换盘符自愈）。
    if app_dir.join("clash-edge-core.exe").exists() {
        return (true, Some(app_dir), false);
    }

    // App/ 目录本身存在 → 仍视为便携。即使 portable.dat 与内核都缺失
    // （部分解压、杀软拦截 34MB 内核、或新旧包文件混用），也走便携分支，
    // 报「App/clash-edge-core.exe 缺失」的明确提示；否则会误判为安装版、
    // 去查 sidecar 目录，报出「Expected sidecar...」的困惑文案（用户实际踩中）。
    if app_dir.is_dir() {
        return (true, Some(app_dir), false);
    }

    (false, None, false)
}

/// 获取应用数据目录
/// 优先级：`CLASH_EDGE_DATA_DIR` 环境变量（参考包启动器布局）→ 便携 Data/ → Tauri 默认
pub fn get_app_data_dir(app_handle: &tauri::AppHandle) -> Result<PathBuf> {
    // 参考包布局：启动器已把数据目录用环境变量指向包根 Data/。
    // 内层 exe 在 App\ClashEdge\ 子目录，无法靠自身位置推断，必须优先用环境变量。
    if let Some(dir) = env_data_dir() {
        std::fs::create_dir_all(&dir)?;
        return Ok(dir);
    }

    // 原生便携模式：Data/ 在 exe 同目录
    if is_portable_mode() {
        if let Some(root) = portable_root() {
            let path = root.join("Data");
            std::fs::create_dir_all(&path)?;
            return Ok(path);
        }
    }

    // 非便携模式：Tauri 默认 app_data_dir
    let path = app_handle.path().app_data_dir()?;
    std::fs::create_dir_all(&path)?;
    Ok(path)
}

/// 获取资源目录（Tauri 打包后的 resource_dir，安装版 sidecar 的父目录）
pub fn get_resource_dir(app_handle: &tauri::AppHandle) -> Result<PathBuf> {
    let path = app_handle.path().resource_dir()?;
    Ok(path)
}

/// 获取 sidecar 目录（安装版资源目录下的 `sidecar/`）。
/// 仅用于非便携安装版；便携模式下 mihomo 固定走 `App/`。
pub fn get_sidecar_dir(app_handle: &tauri::AppHandle) -> Result<PathBuf> {
    Ok(get_resource_dir(app_handle)?.join("sidecar"))
}

/// 获取 mihomo（clash-edge-core / mihomo-win64）可执行文件路径。
///
/// 兼容两套便携布局：
/// - 参考包（C# 启动器）：内层 exe 在 `App\ClashEdge\`，sidecar 名 `mihomo-win64.exe`
///   位于 `App\ClashEdge\sidecar\`（= exe 旁的 `sidecar/`，Tauri resource_dir 子目录）；
/// - 原生便携：mihomo 在 `<root>/App/clash-edge-core.exe`。
/// 以及非便携安装版：`resource_dir/sidecar/clash-edge-core.exe`。
///
/// 依次尝试候选路径，命中即返回；全部缺失才报错（不再回退 %APPDATA%）。
pub fn get_mihomo_path(app_handle: &tauri::AppHandle) -> Result<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    // 候选 1/2：参考包布局——exe 旁 sidecar/ 目录，两种内核名都认。
    // （内层 exe 位于 App\ClashEdge\，sidecar 与之同目录。）
    if let Some(exe_dir) = current_exe_normalized().and_then(|e| e.parent().map(|p| p.to_path_buf()))
    {
        candidates.push(exe_dir.join("sidecar").join("mihomo-win64.exe"));
        candidates.push(exe_dir.join("sidecar").join("clash-edge-core.exe"));
    }

    // 候选 3：原生便携布局——包根 App/ 下。
    if is_portable_mode() {
        if let Some(root) = portable_root() {
            candidates.push(root.join("App").join("clash-edge-core.exe"));
            candidates.push(root.join("App").join("mihomo-win64.exe"));
        }
    }

    // 候选 4：Tauri 安装版 resource_dir/sidecar/。
    if let Ok(sidecar_dir) = get_sidecar_dir(app_handle) {
        candidates.push(sidecar_dir.join("clash-edge-core.exe"));
        candidates.push(sidecar_dir.join("mihomo-win64.exe"));
    }

    for path in candidates {
        if path.exists() {
            return Ok(path);
        }
    }

    Err(mihomo_missing_error(app_handle))
}

/// 生成「mihomo 二进制未找到」的可操作错误。
/// - 参考包启动器布局：提示检查 exe 旁 `sidecar/`（mihomo-win64.exe / clash-edge-core.exe）。
/// - 原生便携：提示检查 App/clash-edge-core.exe 是否随包分发。
/// - 安装版：提示 Tauri 打包是否携带 sidecar。
pub fn mihomo_missing_hint(app_handle: &tauri::AppHandle) -> String {
    if env_data_dir().is_some() {
        let data = env_data_dir().unwrap_or_default();
        let root = data.parent().map(|p| p.display().to_string()).unwrap_or_default();
        format!(
            "mihomo not found in portable (launcher) mode (root: {}).\n\
             Expected App/ClashEdge/sidecar/mihomo-win64.exe (or clash-edge-core.exe) next to\n\
             the inner ClashEdge.exe, but none was found.\n\
             This usually means the package was not extracted completely, or an anti-virus\n\
             quarantined the core binary.\n\
             Re-extract the whole package (ClashEdge.exe + App/ + Data/ + Other/) into the\n\
             same folder and try again.",
            root
        )
    } else if is_portable_mode() {
        let root = portable_root()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        format!(
            "mihomo not found in portable mode (root: {}).\n\
             Expected App/clash-edge-core.exe next to ClashEdge.exe, but it is missing.\n\
             This usually means the package was not extracted completely, or an anti-virus\n\
             quarantined the core binary.\n\
             Re-extract the whole package (ClashEdge.exe + App/ + Data/ + Other/)\n\
             into the same folder and try again.",
            root
        )
    } else {
        let sidecar_dir = get_sidecar_dir(app_handle)
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        // 诊断：安装版模式下没找到 App/（便携布局缺失）。提示可能是只复制了
        // ClashEdge.exe 而没有把整个便携包（App/、Data/、Other/）一起复制，
        // 或者用户运行的是旧版安装布局（sidecar/）而非便携包。
        let root = portable_root()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let app_missing_hint = {
            let app_dir = std::path::Path::new(&root).join("App");
            if !app_dir.exists() {
                format!(
                    "\n\nNote: no App/ folder found next to ClashEdge.exe ({}).\n\
                     This looks like only the exe was copied. Copy the whole portable package\n\
                     (ClashEdge.exe + App/ + Data/ + Other/) into the same folder.",
                    root
                )
            } else {
                String::new()
            }
        };
        format!(
            "mihomo not found in installation mode.\n\
             Expected Tauri sidecar clash-edge-core.exe (or mihomo-win64.exe) in {},\n\
             but it is missing. Verify the build includes sidecar resources.{}",
            sidecar_dir, app_missing_hint
        )
    }
}

/// 供日志/诊断输出：当前到底判定为哪种布局（launcher 参考包 / 原生便携 / 安装版），
/// 以及数据目录/便携根。仅用于给用户/us 排查时定位「是哪个目录、哪条分支在报
/// mihomo 缺失」，不改变任何路径判定逻辑。
pub fn portable_mode_diagnostic() -> String {
    if let Some(data) = env_data_dir() {
        let root = data.parent().map(|p| p.display().to_string()).unwrap_or_default();
        format!(
            "portable (launcher) mode; CLASH_EDGE_DATA_DIR = {}, root = {}",
            data.display(),
            root
        )
    } else if is_portable_mode() {
        let root = portable_root()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        format!("portable mode; root exe dir = {}", root)
    } else {
        format!(
            "INSTALLATION mode (no CLASH_EDGE_DATA_DIR, and no portable marker App/portable.dat \
             or App/clash-edge-core.exe next to this exe at {:?})",
            std::env::current_exe().ok()
        )
    }
}

fn mihomo_missing_error(app_handle: &tauri::AppHandle) -> Error {
    Error::NotFound(mihomo_missing_hint(app_handle))
}

/// 获取配置文件目录
pub fn get_profiles_dir(app_handle: &tauri::AppHandle) -> Result<PathBuf> {
    let data_dir = get_app_data_dir(app_handle)?;
    let profiles_dir = data_dir.join("profiles");
    std::fs::create_dir_all(&profiles_dir)?;
    Ok(profiles_dir)
}

/// 获取日志目录
pub fn get_logs_dir(app_handle: &tauri::AppHandle) -> Result<PathBuf> {
    let data_dir = get_app_data_dir(app_handle)?;
    let logs_dir = data_dir.join("logs");
    std::fs::create_dir_all(&logs_dir)?;
    Ok(logs_dir)
}

/// 获取 GeoData 目录
pub fn get_geodata_dir(app_handle: &tauri::AppHandle) -> Result<PathBuf> {
    let data_dir = get_app_data_dir(app_handle)?;
    let geodata_dir = data_dir.join("geodata");
    std::fs::create_dir_all(&geodata_dir)?;
    Ok(geodata_dir)
}

/// 获取 GeoIP 文件路径
pub fn get_geoip_path(app_handle: &tauri::AppHandle) -> Result<PathBuf> {
    let geodata_dir = get_geodata_dir(app_handle)?;
    Ok(geodata_dir.join("geoip.dat"))
}

/// 获取 GeoSite 文件路径
pub fn get_geosite_path(app_handle: &tauri::AppHandle) -> Result<PathBuf> {
    let geodata_dir = get_geodata_dir(app_handle)?;
    Ok(geodata_dir.join("geosite.dat"))
}

/// 在资源管理器中打开目录
pub fn open_in_explorer<P: AsRef<Path>>(path: P) -> Result<()> {
    let path = path.as_ref();
    if path.exists() {
        #[cfg(target_os = "windows")]
        {
            std::process::Command::new("explorer").arg(path).spawn()?;
        }
        #[cfg(target_os = "macos")]
        {
            std::process::Command::new("open").arg(path).spawn()?;
        }
        #[cfg(target_os = "linux")]
        {
            std::process::Command::new("xdg-open").arg(path).spawn()?;
        }
    }
    Ok(())
}

/// 校验并净化 Profile 名称：拒绝空、`.`/`..`、路径分隔符、盘符、
/// Windows 非法字符、控制符与保留名。所有 profile 命令的
/// `profiles/<name>.yaml` 路径构造都必须先过此校验（防路径穿越）。
pub fn sanitize_profile_name(name: &str) -> Result<String> {
    use crate::util::error::Error;

    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(Error::InvalidArgument(
            "Profile name cannot be empty".to_string(),
        ));
    }
    if trimmed == "." || trimmed == ".." {
        return Err(Error::InvalidArgument("Invalid profile name".to_string()));
    }
    // 路径分隔符 / 盘符（含 Windows 反斜杠与冒号）
    if trimmed.contains('/') || trimmed.contains('\\') || trimmed.contains(':') {
        return Err(Error::InvalidArgument(
            "Profile name cannot contain path separators".to_string(),
        ));
    }
    // Windows 非法文件名字符
    for c in ['<', '>', '"', '|', '?', '*'] {
        if trimmed.contains(c) {
            return Err(Error::InvalidArgument(format!(
                "Profile name cannot contain '{}'",
                c
            )));
        }
    }
    if trimmed.chars().any(|c| c.is_control()) {
        return Err(Error::InvalidArgument(
            "Profile name cannot contain control characters".to_string(),
        ));
    }
    // Windows 会在创建文件时剥离尾点（"foo." 与 "foo" 指向同一文件），
    // 拒绝这类名称避免与既有 Profile 碰撞/覆盖。
    // （尾空格已被上方 trim 规范化，无碰撞面。）
    if trimmed.ends_with('.') {
        return Err(Error::InvalidArgument(
            "Profile name cannot end with '.'".to_string(),
        ));
    }
    // Windows 保留名（大小写不敏感；CON, PRN, AUX, NUL, COM1-9, LPT1-9）
    let upper = trimmed.to_uppercase();
    let base = upper.split('.').next().unwrap_or("");
    const RESERVED: &[&str] = &[
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if RESERVED.contains(&base) {
        return Err(Error::InvalidArgument(format!(
            "'{}' is a reserved Windows name",
            trimmed
        )));
    }
    Ok(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 每个用例独立临时目录（cargo test 并行跑，共用一个进程 PID，
    /// 必须用 tag 区分避免互相踩踏）。
    fn temp_root(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "clash-edge-portable-test-{}-{}",
            std::process::id(),
            tag
        ))
    }

    #[test]
    fn portable_indicators_marker_only() {
        let root = temp_root("marker");
        let app = root.join("App");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(app.join("portable.dat"), b"").unwrap();

        let (is_portable, app_dir, marker_existed) = portable_indicators(Some(&root));
        assert!(is_portable);
        assert_eq!(app_dir, Some(app));
        assert!(marker_existed);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn portable_indicators_self_heal_without_marker() {
        // 复制/改名后 portable.dat 丢失，但内核在 App/ 里 → 仍判定便携
        let root = temp_root("self-heal");
        let app = root.join("App");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(app.join("clash-edge-core.exe"), b"not-real").unwrap();

        let (is_portable, app_dir, marker_existed) = portable_indicators(Some(&root));
        assert!(is_portable);
        assert_eq!(app_dir, Some(app));
        assert!(!marker_existed);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn portable_indicators_false_when_no_evidence() {
        let root = temp_root("no-evidence");
        std::fs::create_dir_all(&root).unwrap();

        let (is_portable, app_dir, marker_existed) = portable_indicators(Some(&root));
        assert!(!is_portable);
        assert_eq!(app_dir, None);
        assert!(!marker_existed);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn portable_indicators_false_when_exe_dir_unknown() {
        let (is_portable, app_dir, marker_existed) = portable_indicators(None);
        assert!(!is_portable);
        assert_eq!(app_dir, None);
        assert!(!marker_existed);
    }

    #[test]
    fn portable_indicators_app_dir_present_counts_as_portable() {
        // 有 App/ 目录但无 marker 也无内核（部分解压/杀软拦截）→ 仍判定便携，
        // 避免误判安装版报 sidecar 困惑文案；缺内核由便携分支给明确提示。
        let root = temp_root("app-no-core");
        let app = root.join("App");
        std::fs::create_dir_all(&app).unwrap();

        let (is_portable, app_dir, _marker) = portable_indicators(Some(&root));
        assert!(is_portable);
        assert_eq!(app_dir, Some(app));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// `env_data_dir` 解析参考包启动器设置的环境变量（指向包根 Data/）。
    #[test]
    fn env_data_dir_parses_launcher_layout() {
        let data = temp_root("env-data").join("Data");
        unsafe {
            std::env::set_var("CLASH_EDGE_DATA_DIR", &data);
        }
        assert_eq!(env_data_dir(), Some(data.clone()));
        // portable_root 取 data 的父目录（包根）
        assert_eq!(portable_root(), Some(data.parent().unwrap().to_path_buf()));
        // 环境变量存在即视为便携
        assert!(is_portable_mode());
        unsafe {
            std::env::remove_var("CLASH_EDGE_DATA_DIR");
        }
        let _ = std::fs::remove_dir_all(data.parent().unwrap());
    }

    /// 尾点在 Windows 上会被剥离（"foo." 与 "foo" 同文件），必须拒绝；
    /// 尾空格已被 trim 规范化，无需拒绝。
    #[test]
    fn sanitize_profile_name_rejects_trailing_dot() {
        assert!(sanitize_profile_name("foo.").is_err());
        assert!(sanitize_profile_name("foo..").is_err());
        assert!(sanitize_profile_name("foo .").is_err());
        assert!(sanitize_profile_name("foo").is_ok());
        assert!(sanitize_profile_name("foo.bar").is_ok());
        assert!(sanitize_profile_name("foo ").is_ok());
    }
}
