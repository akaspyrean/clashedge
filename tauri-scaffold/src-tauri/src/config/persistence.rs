// src-tauri/src/config/persistence.rs
//! 配置文件读写与 ConfigManager
//! 负责：读取 Data/config.yaml、原子写入、旧版配置迁移、对外命令的数据源
//!
//! 单一数据源：`ConfigManager` 持有一个 `Arc<RwLock<Config>>`，
//! `CoreManager`（mihomo 运行时）共享同一个 Arc。所有读取/修改都走同一把锁，
//! 任何修改必须同时完成"内存修改 + 原子落盘"（见 `set_config` 等）。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::RwLock;
use tracing::{debug, info, warn};

use crate::config::model::Config;
use crate::util::atomic::atomic_write;
use crate::util::error::{Error, Result};

/// 配置管理器：持有数据目录与当前配置（共享 Arc），作为 commands 的统一入口
pub struct ConfigManager {
    data_dir: PathBuf,
    config: Arc<RwLock<Config>>,
}

impl ConfigManager {
    pub fn new() -> Self {
        Self {
            data_dir: PathBuf::new(),
            config: Arc::new(RwLock::new(Config::default())),
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
    pub fn init(&mut self, data_dir: &Path) -> Result<()> {
        self.data_dir = data_dir.to_path_buf();
        let loaded = read_config(&self.config_path())?;
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
        info!("ConfigManager initialized at {}", data_dir.display());
        Ok(())
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
    pub fn set_config(&mut self, config: Config) -> Result<()> {
        let mut config = config;
        self.ensure_secure_secret(&mut config);
        *self.config.write() = config;
        self.save()
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

    /// 从前端传入的 JSON 更新配置
    pub fn update_config(&mut self, value: serde_json::Value) -> Result<()> {
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
    pub fn import_config(&mut self, yaml: String) -> Result<()> {
        let config: Config = serde_yaml::from_str(&yaml)
            .map_err(|e| Error::ConfigParse(format!("Invalid config YAML: {}", e)))?;
        // C7 控制器地址限回环（导入 YAML 为用户可控输入，落盘前校验）
        crate::config::model::validate_external_controller(&config.proxy.external_controller)?;
        self.set_config(config)
    }

    /// 当前配置文件路径
    pub fn config_path(&self) -> PathBuf {
        self.data_dir.join("config.yaml")
    }

    fn save(&self) -> Result<()> {
        let config = self.config.read().clone();
        write_config(&self.config_path(), &config)
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

/// 读取配置：文件不存在返回默认；解析失败尝试迁移后再解析
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
            debug!("YAML parse failed ({}), attempting migration", parse_err);
            match crate::config::migration::migrate(config_path) {
                Ok(info) => {
                    info!(
                        "Config migrated {} -> {}",
                        info.current_version, info.target_version
                    );
                    // 迁移会写入新格式，重新读取
                    let migrated = std::fs::read_to_string(config_path)?;
                    let migrated = strip_utf8_bom(&migrated);
                    serde_yaml::from_str::<Config>(migrated)
                        .map_err(|e| Error::ConfigParse(format!("Migrated config invalid: {}", e)))
                }
                Err(_) => {
                    warn!("Migration failed, falling back to default config");
                    Ok(Config::default())
                }
            }
        }
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
}
