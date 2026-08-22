// src-tauri/src/config/persistence.rs
//! 配置文件读写与 ConfigManager
//! 负责：读取 Data/config.yaml、原子写入、旧版配置迁移、对外命令的数据源
//!
//! 单一数据源：`ConfigManager` 持有一个 `Arc<RwLock<Config>>`，
//! `CoreManager`（mihomo 运行时）共享同一个 Arc。所有读取/修改都走同一把锁，
//! 任何修改必须同时完成"内存修改 + 原子落盘"（见 `set_config` 等）。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use parking_lot::RwLock;
use tracing::{error, info, warn};

use crate::config::model::Config;
use crate::util::atomic::atomic_write;
use crate::util::error::{Error, Result};

/// 配置管理器：持有数据目录与当前配置（共享 Arc），作为 commands 的统一入口
pub struct ConfigManager {
    data_dir: PathBuf,
    config: Arc<RwLock<Config>>,
    /// P0-1：启动降级标志。原配置解析且迁移失败时置位：
    /// 内存使用默认配置（应用可用、可进"配置修复状态"），
    /// 磁盘上的原始文件保持不动；用户显式保存时才覆盖。
    degraded: AtomicBool,
}

impl ConfigManager {
    pub fn new() -> Self {
        Self {
            data_dir: PathBuf::new(),
            config: Arc::new(RwLock::new(Config::default())),
            degraded: AtomicBool::new(false),
        }
    }

    /// 初始化：设置数据目录并读取磁盘上的配置。
    /// 读取后立即做运行时语义校验（mode / find-process-mode / profile 兜底），
    /// 保证喂给 mihomo 的配置永远合法（导入/迁移进来的非法值在这里被修正）。
    ///
    /// 控制器密钥：首次运行（文件不存在 → 默认占位密钥）或旧版配置仍是
    /// 固定默认 "clash-edge-secret" 时，轮换为随机密钥并持久化，避免
    /// 控制器被已知默认密钥接管；已有随机密钥的配置保持不变（无扰动）。
    /// 轮换判定与落盘统一走 `ensure_secure_secret`（H1），init 只保留日志语义。
    ///
    /// P0-1：坏配置绝不静默覆盖。`read_config` 失败（解析失败且迁移失败）
    /// 时不返回错误炸掉启动，而是进入降级模式：内存用默认值让应用能打开
    /// 界面修复，磁盘原始文件保持不动（已由 read_config 做了 .corrupt-*.bak
    /// 备份），直到用户下一次显式保存。
    pub fn init(&mut self, data_dir: &Path) -> Result<()> {
        self.data_dir = data_dir.to_path_buf();
        match read_config(&self.config_path()) {
            Ok(loaded) => {
                self.degraded.store(false, Ordering::SeqCst);
                let mut validated = crate::core::config::merge_rules(loaded);
                let rotated = self.ensure_secure_secret(&mut validated);
                if rotated {
                    info!("Rotating controller secret to a random value");
                    // 轮换必须落盘，否则下次启动又回到占位值、也无法认证重启后的核心。
                    self.set_config(validated)?;
                } else {
                    // 仅内存校正（与旧行为一致：磁盘在下次保存时再归一化）
                    *self.config.write() = validated;
                }
            }
            Err(e) => {
                error!(
                    "Config load failed; entering degraded mode with defaults \
                     (original file preserved on disk): {}",
                    e
                );
                self.degraded.store(true, Ordering::SeqCst);
                // 只改内存，不落盘——绝不自动把坏文件覆盖成默认配置
                let mut validated = crate::core::config::merge_rules(Config::default());
                self.ensure_secure_secret(&mut validated);
                *self.config.write() = validated;
            }
        }
        info!("ConfigManager initialized at {}", data_dir.display());
        Ok(())
    }

    /// 是否处于启动降级模式（原配置损坏且迁移失败，当前内存为默认配置）。
    /// 前端/命令可据此提示用户"检测到无法解析的配置，正在使用默认值，
    /// 原文件已备份，保存设置将写入新配置"。
    pub fn is_degraded(&self) -> bool {
        self.degraded.load(Ordering::SeqCst)
    }

    /// 密钥兜底轮换（H1 收敛）：空 / 固定占位 / 旧版遗留固定密钥 → 随机密钥。
    /// 已是随机密钥（或任何非占位非空值）保持不变，避免误轮换。
    /// 返回是否发生了轮换（init 据此保留原有日志语义）。
    fn ensure_secure_secret(&self, config: &mut Config) -> bool {
        if crate::config::model::needs_secret_rotation(&config.proxy.secret) {
            config.proxy.secret = crate::config::model::generate_random_secret();
            true
        } else {
            false
        }
    }

    /// 获取当前配置快照（克隆，调用方自由持有）
    pub fn get_config(&self) -> Config {
        self.config.read().clone()
    }

    /// 共享配置句柄：交给 CoreManager 等运行时组件，保证单一数据源
    pub fn config_handle(&self) -> Arc<RwLock<Config>> {
        self.config.clone()
    }

    /// 替换配置并立即持久化。
    ///
    /// 所有落盘路径（set_config / update_config / import_config / reset_config /
    /// init 的轮换分支）统一在写盘前强制 `ensure_secure_secret`，保证
    /// reset/import/update 后密钥也绝不为占位/空/旧遗留值。
    ///
    /// P0-2 disk-first：先原子落盘，成功后才提交内存。旧实现"先改内存再
    /// save()"在磁盘写失败时会留下「内存=新值、磁盘=旧值」的分裂状态，
    /// 且后续所有读方都拿到与磁盘不一致的值；反过来（disk-first）失败时
    /// 内存保持旧值、调用方拿到 Err，两边永远一致。
    pub fn set_config(&mut self, config: Config) -> Result<()> {
        let mut config = config;
        self.ensure_secure_secret(&mut config);
        write_config(&self.config_path(), &config)?;
        *self.config.write() = config;
        Ok(())
    }

    /// 就地修改：闭包里改完后统一落盘
    pub fn update_config_with<F>(&mut self, f: F) -> Result<()>
    where
        F: FnOnce(&mut Config),
    {
        let mut config = self.get_config();
        f(&mut config);
        self.set_config(config)
    }

    /// 从前端传入的 JSON 构造新配置（解析 + 校验，**不落盘**）。
    /// 供命令层的事务流程使用：先拿到校验过的新配置，
    /// 再由事务统一执行「持久化 → 应用运行时 → 失败回滚」（P0-3）。
    pub fn prepare_update(&self, value: serde_json::Value) -> Result<Config> {
        let mut config: Config = serde_json::from_value(value)?;
        // C7 控制器地址限回环（用户可控输入，落盘前校验）
        crate::config::model::validate_external_controller(&config.proxy.external_controller)?;
        // P0-3：前端 get_config 返回脱敏 secret（SECRET_REDACTED），前端回传时
        // 保持脱敏值——此时保留现有真实密钥，不轮换。
        // 若前端传入空串也同理保留（前端不应直接操作 secret）。
        if config.proxy.secret == crate::config::model::SECRET_REDACTED
            || config.proxy.secret.is_empty()
        {
            config.proxy.secret = self.get_config().proxy.secret;
        }
        Ok(config)
    }

    /// 从 YAML 字符串构造新配置（解析 + 校验，**不落盘**）。供事务流程使用。
    pub fn prepare_import(&self, yaml: String) -> Result<Config> {
        let config: Config = serde_yaml::from_str(&yaml)
            .map_err(|e| Error::ConfigParse(format!("Invalid config YAML: {}", e)))?;
        // C7 控制器地址限回环（导入 YAML 为用户可控输入，落盘前校验）
        crate::config::model::validate_external_controller(&config.proxy.external_controller)?;
        Ok(config)
    }

    /// 从前端传入的 JSON 更新配置（非事务路径，仅测试与简单场景使用；
    /// 命令层请走 prepare_update + 运行时事务）
    pub fn update_config(&mut self, value: serde_json::Value) -> Result<()> {
        let config = self.prepare_update(value)?;
        self.set_config(config)
    }

    /// 重置为默认配置
    pub fn reset_config(&mut self) -> Result<()> {
        self.set_config(Config::default())
    }

    /// 导出当前配置为 YAML 字符串
    pub fn export_config(&self) -> Result<String> {
        let yaml = serde_yaml::to_string(&*self.config.read())?;
        Ok(yaml)
    }

    /// 从 YAML 字符串导入配置并持久化
    /// （非事务路径；命令层请走 prepare_import + 运行时事务）
    pub fn import_config(&mut self, yaml: String) -> Result<()> {
        let config = self.prepare_import(yaml)?;
        self.set_config(config)
    }

    /// 当前配置文件路径
    pub fn config_path(&self) -> PathBuf {
        self.data_dir.join("config.yaml")
    }
}

impl Default for ConfigManager {
    fn default() -> Self {
        Self::new()
    }
}

/// 去除 UTF-8 BOM。Windows 记事本及部分编辑器保存 UTF-8 时会写 BOM
/// （0xEFBBBF），serde_yaml 不认 BOM 开头，会解析失败进而触发迁移、
/// 最终导致启动 panic（退出码 101）。读取时统一剥离。
pub fn strip_utf8_bom(content: &str) -> &str {
    content.trim_start_matches('\u{feff}')
}

/// 读取配置：文件不存在返回默认；解析失败先备份原文件，再在内存中迁移。
///
/// P0-1 重构后的失败语义：
/// - 迁移成功 → 返回迁移结果（尚未落盘；原文件已被 .corrupt-*.bak 备份，
///   下一次显式保存才会写入新格式）；
/// - 迁移失败 → 返回 Err（由 init 进入降级模式），**绝不返回默认配置
///   冒充成功**——旧行为会用默认值静默覆盖用户配置，属于数据丢失。
pub fn read_config(config_path: &Path) -> Result<Config> {
    if !config_path.exists() {
        info!("Config file not found, returning default config");
        return Ok(Config::default());
    }

    let content = std::fs::read_to_string(config_path)?;
    let content = strip_utf8_bom(&content);

    // 尝试按新格式解析
    match serde_yaml::from_str::<Config>(content) {
        Ok(config) => Ok(config),
        Err(parse_err) => {
            warn!(
                "Config parse failed ({}); backing up original before migration",
                parse_err
            );
            backup_corrupt_config(config_path);
            match crate::config::migration::migrate_content(content) {
                Ok(config) => {
                    info!("Config migrated in memory; new format persists on next save");
                    Ok(config)
                }
                Err(e) => Err(Error::ConfigParse(format!(
                    "config.yaml is invalid and could not be migrated: {} \
                     (original file preserved, see *.corrupt-*.bak)",
                    e
                ))),
            }
        }
    }
}

/// 解析失败时把原配置复制为同目录下的 `config.yaml.corrupt-<时间戳>.bak`。
/// 复制（而非移动）：无论后续迁移/保存成败，用户原始数据都有据可查。
fn backup_corrupt_config(config_path: &Path) {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let file_name = config_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "config.yaml".to_string());
    let backup = config_path.with_file_name(format!("{}.corrupt-{}.bak", file_name, stamp));
    match std::fs::copy(config_path, &backup) {
        Ok(_) => info!("Original config backed up to {}", backup.display()),
        Err(e) => error!(
            "Failed to back up corrupt config to {}: {}",
            backup.display(),
            e
        ),
    }
}

/// 原子写入配置文件：先写临时文件再重命名
///
/// 注意：`config.yaml` 是**应用配置**（AppConfig），完整保留应用级字段
/// （`profile` 激活名、`locale`、`geodata-mode`、`mixin-enabled` 等）。
/// mihomo 实际加载的是由 `core::config::build_runtime_config` 生成的
/// 独立运行时配置（runtime-config.yaml），应用配置不再直接喂给 mihomo，
/// 因此这里不再剔除任何字段——剔除应用级字段属于运行时配置生成阶段。
pub fn write_config(config_path: &Path, config: &Config) -> Result<()> {
    let yaml = serde_yaml::to_string(config)?;
    // 原子写入：随机后缀临时文件 + 排他创建 + rename（见 util::atomic）
    atomic_write(config_path, yaml.as_bytes())?;

    info!("Config written to: {:?}", config_path);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    /// config.yaml 是完整应用配置（AppConfig）：应用级字段
    /// geodata-mode / profile 必须原样落盘（真实激活依赖 profile 持久化），
    /// 同时保留 external-controller / secret / tun / dns 默认值，且能读回。
    #[test]
    fn write_config_keeps_full_app_config() {
        let mut config = Config::default();
        config.general.geodata_mode = serde_yaml::Value::String("metax".to_string());
        config.general.profile = "DIRECT".to_string();
        config.proxy.external_controller = "127.0.0.1:50715".to_string();
        config.proxy.secret = "b1616fdd-63a8-44e9-b196-c63b68307a9b".to_string();

        let dir = std::env::temp_dir().join(format!(
            "clash-edge-persist-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.yaml");

        write_config(&path, &config).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();

        // 应用级字段必须保留（写盘不再剔除）
        assert!(
            content.contains("geodata-mode: metax"),
            "geodata-mode should persist:\n{}",
            content
        );
        let has_top_level_profile = content.lines().any(|l| l.starts_with("profile:"));
        assert!(
            has_top_level_profile,
            "top-level profile must persist for activation:\n{}",
            content
        );
        assert!(
            content.contains("external-controller: 127.0.0.1:50715"),
            "missing controller:\n{}",
            content
        );
        assert!(
            content.contains("secret: b1616fdd-63a8-44e9-b196-c63b68307a9b"),
            "missing secret:\n{}",
            content
        );
        // 默认值必须有效：tun.stack 非空、dns nameserver 非空
        assert!(
            content.contains("stack: system"),
            "tun.stack should default to system:\n{}",
            content
        );
        assert!(
            content.contains("listen: 127.0.0.1:9053"),
            "dns.listen should have a default:\n{}",
            content
        );
        assert!(
            content.contains("223.5.5.5"),
            "dns default-nameserver should be populated:\n{}",
            content
        );

        // 落盘后能读回（往返一致性）
        let loaded = read_config(&path).unwrap();
        assert_eq!(
            loaded.general.geodata_mode,
            serde_yaml::Value::String("metax".to_string())
        );
        assert_eq!(loaded.general.profile, "DIRECT");
        assert_eq!(loaded.proxy.external_controller, "127.0.0.1:50715");
        assert_eq!(loaded.proxy.secret, "b1616fdd-63a8-44e9-b196-c63b68307a9b");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 控制器密钥轮换：无配置文件的首次运行会生成随机密钥并落盘；
    /// 已随机的配置再次 init 不再轮换（避免每次启动抖动配置）。
    #[test]
    fn init_rotates_fixed_default_secret() {
        let dir = std::env::temp_dir().join(format!(
            "clash-edge-secret-rot-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();

        // 1) 无配置文件（全新安装）：生成随机密钥并写盘
        let mut mgr = ConfigManager::new();
        mgr.init(&dir).unwrap();
        let cfg = mgr.get_config();
        assert_ne!(
            cfg.proxy.secret,
            crate::config::model::default_secret_placeholder()
        );
        assert_eq!(
            cfg.proxy.secret.len(),
            32,
            "random secret should be 32 hex chars"
        );
        let written = std::fs::read_to_string(mgr.config_path()).unwrap();
        assert!(
            written.contains(&cfg.proxy.secret),
            "rotated secret must persist:\n{}",
            written
        );

        // 2) 重新 init：已随机后不再轮换，密钥保持不变（无扰动）
        let mut mgr2 = ConfigManager::new();
        mgr2.init(&dir).unwrap();
        let cfg2 = mgr2.get_config();
        assert_eq!(cfg2.proxy.secret, cfg.proxy.secret);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 旧版 0.8.5 配置残留固定默认密钥 → init 时轮换为随机值。
    #[test]
    fn init_rotates_legacy_hardcoded_secret() {
        let dir = std::env::temp_dir().join(format!(
            "clash-edge-secret-legacy-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.yaml");
        let mut cfg = Config::default();
        cfg.proxy.secret = crate::config::model::default_secret_placeholder().to_string();
        write_config(&path, &cfg).unwrap();

        let mut mgr = ConfigManager::new();
        mgr.init(&dir).unwrap();
        let loaded = mgr.get_config();
        assert_ne!(
            loaded.proxy.secret,
            crate::config::model::default_secret_placeholder()
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// UTF-8 BOM 会令 serde_yaml 解析失败并触发迁移（迁移重读仍带 BOM，
    /// 再解析失败）导致启动 panic 退出码 101。read_config 必须剥离 BOM。
    #[test]
    fn read_config_handles_utf8_bom() {
        let mut config = Config::default();
        config.general.mixed_port = 7897;
        let mut buf = Vec::new();
        buf.extend_from_slice(&[0xEF, 0xBB, 0xBF]); // UTF-8 BOM
        buf.extend_from_slice(serde_yaml::to_string(&config).unwrap().as_bytes());

        let dir = std::env::temp_dir().join(format!(
            "clash-edge-bom-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.yaml");
        std::fs::write(&path, &buf).unwrap();

        let loaded = read_config(&path).unwrap();
        assert_eq!(
            loaded.general.mixed_port, 7897,
            "BOM should be stripped on read"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// H1：reset_config 走 set_config 统一兜底——重置后的占位密钥必须被轮换
    /// 为随机密钥（否则重置后控制器仍可被默认密钥接管）。
    #[test]
    fn reset_config_rotates_placeholder_secret() {
        let dir = std::env::temp_dir().join(format!(
            "clash-edge-reset-secret-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut mgr = ConfigManager::new();
        mgr.init(&dir).unwrap();

        mgr.reset_config().unwrap();
        let cfg = mgr.get_config();
        assert_ne!(
            cfg.proxy.secret,
            crate::config::model::default_secret_placeholder(),
            "reset_config must not leave placeholder secret"
        );
        assert_eq!(
            cfg.proxy.secret.len(),
            32,
            "reset_config secret should be random 32 hex"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// H1：import_config 导入缺 secret 的 YAML——缺失键回落到占位值，
    /// 落盘前必须被兜底轮换为非占位随机密钥。
    #[test]
    fn import_config_without_secret_rotates() {
        let dir = std::env::temp_dir().join(format!(
            "clash-edge-import-secret-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut mgr = ConfigManager::new();
        mgr.init(&dir).unwrap();

        mgr.import_config("mixed-port: 7890\nmode: rule\n".to_string())
            .unwrap();
        let cfg = mgr.get_config();
        assert_ne!(
            cfg.proxy.secret,
            crate::config::model::default_secret_placeholder(),
            "import without secret must rotate"
        );
        assert_eq!(cfg.proxy.secret.len(), 32);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// H1：update_config 传入占位 / 空 secret——必须被兜底轮换为非占位随机密钥。
    #[test]
    fn update_config_with_weak_secret_rotates() {
        let dir = std::env::temp_dir().join(format!(
            "clash-edge-update-secret-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut mgr = ConfigManager::new();
        mgr.init(&dir).unwrap();

        // 占位 secret → 轮换
        mgr.update_config(serde_json::json!({
            "mixed-port": 7890,
            "secret": crate::config::model::default_secret_placeholder(),
        }))
        .unwrap();
        let cfg = mgr.get_config();
        assert_ne!(
            cfg.proxy.secret,
            crate::config::model::default_secret_placeholder(),
            "placeholder secret must be rotated"
        );
        assert_eq!(cfg.proxy.secret.len(), 32);

        // 空 secret → 轮换
        mgr.update_config(serde_json::json!({
            "mixed-port": 7890,
            "secret": "",
        }))
        .unwrap();
        let cfg = mgr.get_config();
        assert_ne!(cfg.proxy.secret, "", "empty secret must be rotated");
        assert_eq!(cfg.proxy.secret.len(), 32);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// P0-1：损坏的配置文件 → read_config 返回 Err（绝不返回默认配置冒充成功），
    /// 原文件原样保留，并生成 .corrupt-*.bak 备份。
    #[test]
    fn corrupt_config_errors_and_backs_up() {
        let dir = std::env::temp_dir().join(format!(
            "clash-edge-corrupt-bak-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.yaml");
        let garbage = "app: [unclosed\n  broken";
        std::fs::write(&path, garbage).unwrap();

        assert!(read_config(&path).is_err(), "corrupt config must error");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            garbage,
            "original file must stay untouched"
        );
        let has_backup = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().contains(".corrupt-"));
        assert!(has_backup, "a .corrupt-*.bak backup must exist");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// P0-1：init 遇到坏配置进入降级模式——内存为默认值（应用可用），
    /// 磁盘上的原始文件保持不动。
    #[test]
    fn init_enters_degraded_mode_on_corrupt_config() {
        let dir = std::env::temp_dir().join(format!(
            "clash-edge-degraded-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.yaml");
        let garbage = "app: [unclosed";
        std::fs::write(&path, garbage).unwrap();

        let mut mgr = ConfigManager::new();
        mgr.init(&dir).unwrap();
        assert!(mgr.is_degraded(), "must be in degraded mode");
        // 内存是可用的默认值
        assert_eq!(mgr.get_config().general.mixed_port, 7890);
        // 磁盘原文件未被覆盖
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            garbage,
            "degraded mode must not overwrite the original file"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// P0-1：旧版平铺格式通过内存迁移成功加载（不降级、不丢字段）
    #[test]
    fn init_migrates_legacy_flat_config() {
        let dir = std::env::temp_dir().join(format!(
            "clash-edge-legacy-mig-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.yaml");
        let legacy = "mixed-port: 7897\nallow-lan: true\nexternal-controller: 127.0.0.1:9091\n";
        std::fs::write(&path, legacy).unwrap();

        let mut mgr = ConfigManager::new();
        mgr.init(&dir).unwrap();
        assert!(!mgr.is_degraded());
        let cfg = mgr.get_config();
        assert_eq!(cfg.general.mixed_port, 7897);
        assert!(cfg.general.allow_lan);
        assert_eq!(cfg.proxy.external_controller, "127.0.0.1:9091");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// P0-2 disk-first：set_config 落盘失败时内存必须保持旧值。
    /// 通过把 config.yaml 替换成目录来制造写入失败（rename 到目录路径必然失败）。
    #[test]
    fn set_config_disk_failure_keeps_memory_consistent() {
        let dir = std::env::temp_dir().join(format!(
            "clash-edge-diskfirst-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut mgr = ConfigManager::new();
        mgr.init(&dir).unwrap();

        // 用目录顶掉 config.yaml，使后续原子写入失败
        std::fs::remove_file(mgr.config_path()).unwrap();
        std::fs::create_dir_all(mgr.config_path()).unwrap();

        let mut new_cfg = mgr.get_config();
        new_cfg.general.mixed_port = 8080;
        assert!(
            mgr.set_config(new_cfg).is_err(),
            "write to a directory path must fail"
        );
        assert_eq!(
            mgr.get_config().general.mixed_port,
            7890,
            "memory must keep the old value when the disk write fails"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
