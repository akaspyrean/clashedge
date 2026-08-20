# ClashEdge → Tauri 2 重构实施方案

> **约束**：仅重构现有功能，不新增 UI/功能；参数与翻译集中配置；以稳定、安全、高效、轻量为主
> **参考实现**：clash-verge-rev (22k★)、clash-nyanpasu (13k★) —— 均为 Tauri 2 + Rust 后端 + Web 前端架构

---

## 1. 现状与差距分析

| 维度 | 现状 (R8.3 / CFW 0.20.39 基线) | Tauri 2 目标 |
|------|-------------------------------|--------------|
| **运行时体积** | ~157 MB (`ClashEdge.exe` 含 Electron + Chromium) | **10–20 MB** (系统 WebView2 + Rust) |
| **前端框架** | Vue 2.7.14 + Vuex + vue-router + vue-electron（仅存 3.4 MB 混淆 bundle，无源码） | **Vue 3.5 + Pinia + Vue Router 4 + Vite**（从零重写，功能对等） |
| **主进程** | Node.js (Electron main.js + 3 个 IIFE 补丁) | **Rust** (`tauri::command` + `sidecar` 二进制管理) |
| **安全模型** | `nodeIntegration:true, contextIsolation:false`（渲染层直连 Node） | **强制隔离**：前端纯 Web，仅通过类型安全 IPC 与 Rust 通信 |
| **核心内核** | `mihomo-win64.exe` 子进程，启动/停止/配置均在 main.js 硬编码 | **Rust sidecar**：编译期嵌入，运行时托管，配置热重载 |
| **配置体系** | YAML 分散 (`config.yaml` + `profile-preprocessor.cjs` 预处理) | **集中化**：单一 `Config` 结构体 → 序列化为 YAML/JSON，前端只读/写这一份 |
| **国际化** | 硬编码字符串散落在 renderer bundle | **集中化**：`i18n/` 目录下 `zh-CN.yaml`, `en-US.yaml` 等，前端 `vue-i18n` / 后端 `fluent` 双端复用 |
| **系统托盘** | Electron `Tray` + 动态菜单构建 (main.js ~200 行) | **tauri-plugin-tray-icon** + `tauri::menu`，菜单定义集中在 Rust |
| **系统代理/TUN** | `EnableLoopback.exe` + `schtasks.xml` + `wintun.dll` + `go-tun2socks.exe` 手工调度 | **tauri-plugin-network** / 自建 Rust 模块，调用 Windows API / `wintun` crate |
| **地理数据更新** | main.js IIFE hook：双源下载 + 原子替换 + rollback | **Rust 侧命令**：复用相同逻辑，前端仅触发 `invoke('geodata_update')` |
| **静默自启动** | C# Launcher + Electron `setLoginItemSettings` patch | **tauri-plugin-autostart** + `AppHandle::startup` |
| **便携数据目录** | C# 启动器建立 Junction (`App/data` → `Data/`) | **Rust 侧 `AppHandle::path().app_data_dir()`** 自定义，或保留 Launcher 负责目录结构 |

---

## 2. 总体架构

```
ClashEdge (Tauri 2)
├── src-tauri/                          # Rust 后端
│   ├── Cargo.toml
│   ├── tauri.conf.json                 # 窗口、图标、权限、sidecar、插件
│   ├── build.rs                        # 编译期嵌入 mihomo/wintun/go-tun2socks/EnableLoopback
│   ├── src/
│   │   ├── main.rs                     # 入口、插件注册、菜单/托盘构建
│   │   ├── core/                       # mihomo 生命周期管理
│   │   │   ├── manager.rs              # 启动/停止/重载/健康检查/版本查询
│   │   │   ├── config.rs               # 配置模板渲染 → 生成最终 YAML
│   │   │   └── sidecar.rs              # sidecar 二进制路径解析、参数构造
│   │   ├── proxy/                      # 系统代理 / TUN / 回环
│   │   │   ├── system_proxy.rs         # WinHTTP / 注册表 / EnableLoopback 调用
│   │   │   ├── tun.rs                  # wintun crate 封装
│   │   │   └── loopback.rs             # EnableLoopback.exe 封装
│   │   ├── geodata/                    # GeoIP/GeoSite 更新
│   │   │   ├── updater.rs              # 双源下载、原子替换、回滚
│   │   │   └── sources.rs              # URL 列表、校验哈希
│   │   ├── config/                     # 统一配置模型
│   │   │   ├── model.rs                # Config, Profile, Proxy, RuleSet, Dns, Tun, ...
│   │   │   ├── persistence.rs          # 读/写 `Data/config.yaml` + `Data/profiles/*.yaml`
│   │   │   └── migration.rs            # 旧版配置 → 新版结构
│   │   ├── i18n/                       # 翻译资源（后端也需用：托盘菜单、通知）
│   │   │   ├── loader.rs               # 读取 `resources/i18n/*.ftl` 或 `*.yaml`
│   │   │   └── macros.rs               # `t!()` 宏 / `fluent` bundle
│   │   ├── commands/                   # #[tauri::command] 入口
│   │   │   ├── core.rs                 # start/stop/restart/status/version
│   │   │   ├── config.rs               # get/set/import/export/validate
│   │   │   ├── proxy.rs                # system_proxy/tun/loopback toggle
│   │   │   ├── geodata.rs              # update/check_version
│   │   │   ├── profiles.rs             # list/create/delete/activate/rename
│   │   │   └── tray.rs                 # menu_update/set_icon
│   │   ├── tray/                       # 托盘菜单构建
│   │   │   ├── builder.rs              # 动态菜单：代理组、模式、连接数
│   │   │   └── events.rs               # 点击/右键/图标更新
│   │   └── util/                       # 路径、日志、错误类型
│   │       ├── paths.rs                # 便携目录解析（兼容 Launcher 传入的 CLASH_EDGE_DATA_DIR）
│   │       ├── logging.rs              # tracing + tauri-plugin-log
│   │       └── error.rs                # ClashError → 前端友好序列化
│   └── resources/
│       ├── i18n/                       # 翻译源文件
│       │   ├── zh-CN.yaml
│       │   └── en-US.yaml
│       ├── mihomo-win64.exe            # sidecar (通过 build.rs 复制)
│       ├── wintun.dll
│       ├── go-tun2socks.exe
│       ├── EnableLoopback.exe
│       └── schtasks.xml
│
├── src/                                # Vue 3 前端 (Vite + TypeScript)
│   ├── main.ts                         # 入口、Pinia、Router、i18n 注册
│   ├── App.vue                         # 根布局（含全局 Toast/Loading/Confirm）
│   ├── router/                         # 路由表：General / Proxies / Profiles / Rules / Connections / Logs / Settings / About
│   ├── stores/                         # Pinia stores
│   │   ├── core.ts                     # coreStatus, version, uptime
│   │   ├── config.ts                   # 当前配置对象、脏标记、保存动作
│   │   ├── profiles.ts                 # 列表、激活项、导入导出
│   │   ├── proxies.ts                  # 代理组、延迟、选中项
│   │   ├── connections.ts              # 实时连接表
│   │   ├── logs.ts                     # 日志流、级别过滤
│   │   ├── geodata.ts                  # 版本、更新状态
│   │   └── ui.ts                       # 主题、侧边栏折叠、语言、通知队列
│   ├── components/
│   │   ├── layout/                     # Sidebar, Header, TrayMenuMirror
│   │   ├── common/                     # Button, Card, Table, Modal, Select, Tooltip, Badge, Icon
│   │   ├── proxies/                    # ProxyGroupCard, ProxySelector, LatencyChart
│   │   ├── profiles/                   # ProfileList, ProfileEditor (Monaco), ProfileImport
│   │   ├── rules/                      # RuleSetTable, RuleEditor
│   │   ├── connections/                # ConnectionTable, ConnectionFilter
│   │   ├── logs/                       # LogViewer, LogFilter
│   │   └── settings/                   # SettingsTabs (General/Proxy/TUN/Advanced/About)
│   ├── composables/                    # 复用逻辑
│   │   ├── useIpc.ts                   # 类型安全 invoke 封装
│   │   ├── useTray.ts                  # 托盘菜单同步
│   │   ├── useTheme.ts                 # 暗/亮/跟随系统
│   │   └── useI18n.ts                  # 语言切换、本地化工具
│   ├── i18n/                           # 前端翻译（与后端 resources/i18n 同源）
│   │   ├── index.ts                    # createI18n({legacy:false, messages:{...}})
│   │   ├── zh-CN.yaml
│   │   └── en-US.yaml
│   ├── styles/                         # 全局样式、变量、主题
│   │   ├── variables.css               # CSS 变量（颜色、间距、圆角、阴影）
│   │   ├── global.css
│   │   └── theme.css                   # [data-theme] 切换
│   └── assets/                         # 图标、字体、logo
│
├── package.json                        # 前端依赖、scripts
├── vite.config.ts                      # Vite + Vue + TypeScript + Tauri 插件
├── tsconfig.json
└── index.html
```

---

## 3. 关键模块设计

### 3.1 统一配置模型 (`src-tauri/src/config/model.rs`)

```rust
// 单一事实来源：前端只读/写此结构，后端序列化为 mihomo YAML
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "kebab-case")]
pub struct Config {
    pub mixed_port: u16,
    pub allow_lan: bool,
    pub log_level: LogLevel,
    pub ipv6: bool,
    pub geodata_mode: bool,
    pub geodata_loader: String,
    pub geo_auto_update: bool,
    pub geox_url: GeoXUrl,
    pub find_process_mode: String,
    pub sniffer: Sniffer,
    pub dns: DnsConfig,
    pub tun: Option<TunConfig>,
    pub proxies: Vec<Proxy>,
    pub proxy_groups: Vec<ProxyGroup>,
    pub rule_providers: HashMap<String, RuleProvider>,
    pub rules: Vec<String>,
    // UI-only 字段（不写入 mihomo）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ui: Option<UiConfig>,
}
```

- **前端**：`config.ts` store 持有 `Config`，修改后 `invoke('config_save', {config})`
- **后端**：`config_save` → 校验 → `persistence::write_config()` → `core::manager::reload()` 热重载 mihomo
- **迁移**：`migration.rs` 读取旧 `Data/config.yaml` + `profile-preprocessor.cjs` 逻辑，一次性转换为新结构

### 3.2 国际化集中化

```
src-tauri/resources/i18n/
├── zh-CN.yaml
└── en-US.yaml
```

```yaml
# zh-CN.yaml 示例
app:
  name: "ClashEdge"
  version: "版本 {version}"
tray:
  control_panel: "控制面板"
  system_proxy: "系统代理"
  tun_mode: "TUN 模式"
  config_mixin: "配置混入"
  proxy_mode: "代理模式"
  mode_global: "全局"
  mode_rule: "规则"
  mode_direct: "直连"
  mode_script: "脚本"
  proxy_groups: "代理组"
  connections: "连接"
  close_all: "关闭全部"
  more: "更多"
  dev_tools: "切换开发者工具"
  move_to_monitor: "移动主面板到最近显示器"
  restart: "重启"
  force_quit: "强制退出"
  quit: "退出"
geodata:
  title: "地理数据"
  subtitle: "GeoSite / GeoIP · 仅手动更新"
  update_btn: "手动更新"
  updating: "正在更新…"
  done: "更新完成"
  restart_prompt: "地理数据已更新。现在重启应用以加载新数据？"
  fail_permission: "临时文件被占用。请关闭其他 ClashEdge 实例后重试。"
  fail_timeout: "下载超时。请确认 Mihomo 已启动且网络可用后重试。"
  fail_network: "下载未完成。请检查网络后重试。"
settings:
  tabs:
    general: "常规"
    proxy: "代理"
    tun: "TUN"
    advanced: "高级"
    about: "关于"
  language: "语言"
  theme: "主题"
  theme_dark: "深色"
  theme_light: "浅色"
  theme_system: "跟随系统"
  autostart: "开机自启"
  silent_autostart: "静默启动(不显示主窗口)"
  data_dir: "数据目录"
  open_data_dir: "打开数据目录"
  core_version: "核心版本"
  check_update: "检查更新"
```

- **后端**：`i18n::loader::load(lang)` → `fluent::FluentBundle` → `t!("tray.system-proxy")` 用于托盘菜单、系统通知
- **前端**：`i18n/index.ts` 读取同一组 YAML（或构建时同步生成 JSON）→ `vue-i18n` → `t('tray.system_proxy')`
- **单一来源**：CI 步骤校验两端 key 一致

### 3.3 Core 管理 (`core/manager.rs`)

```rust
pub struct CoreManager {
    child: Option<Child>,
    config: Arc<RwLock<Config>>,
    status: Arc<RwLock<CoreStatus>>,
    port: u16,
    secret: String,
}

impl CoreManager {
    pub async fn start(&mut self, config: &Config) -> Result<()> { ... }
    pub async fn stop(&mut self) -> Result<()> { ... }
    pub async fn reload(&self) -> Result<()> { ... }  // SIGHUP 或重写配置文件+信号
    pub async fn version(&self) -> Result<String> { ... }  // `mihomo -v`
    pub fn status(&self) -> CoreStatus { ... }
    pub fn api_client(&self) -> Result<ApiClient> { ... }  // 外部控制器 REST
}
```

- **sidecar 路径**：`tauri::api::path::resource_dir().join("mihomo-win64.exe")`
- **工作目录**：`Data/`（含 `config.yaml`、GeoIP/GeoSite、`wintun.dll`）
- **健康检查**：定时 `GET /version`，失败自动重启（可配置）

### 3.4 托盘菜单动态构建 (`tray/builder.rs`)

```rust
pub fn build_tray_menu(
    app: &AppHandle,
    core_status: CoreStatus,
    proxies: &[ProxyGroupInfo],
    config: &Config,
    i18n: &I18n,
) -> Result<Menu> {
    let mut menu = Menu::new();
    // 固定项
    menu.append_items(&[
        MenuItem::with_id(app, "control_panel", i18n.t("tray.control_panel"), true, None::<&str>)?,
        PredefinedMenuItem::separator(app)?,
        // 系统代理 / TUN / 混入
        CheckMenuItem::with_id(app, "system_proxy", i18n.t("tray.system_proxy"), true, config.system_proxy_enabled, None)?,
        CheckMenuItem::with_id(app, "tun_mode", i18n.t("tray.tun_mode"), true, config.tun_enabled, None)?,
        CheckMenuItem::with_id(app, "config_mixin", i18n.t("tray.config_mixin"), true, config.mixin_enabled, None)?,
        PredefinedMenuItem::separator(app)?,
        // 代理模式
        Submenu::with_id_and_items(app, "proxy_mode", i18n.t("tray.proxy_mode"), true, &[
            RadioMenuItem::with_id(app, "mode_global", i18n.t("tray.mode_global"), true, config.mode == "global", None)?,
            RadioMenuItem::with_id(app, "mode_rule", i18n.t("tray.mode_rule"), true, config.mode == "rule", None)?,
            RadioMenuItem::with_id(app, "mode_direct", i18n.t("tray.mode_direct"), true, config.mode == "direct", None)?,
            RadioMenuItem::with_id(app, "mode_script", i18n.t("tray.mode_script"), true, config.mode == "script", None)?,
        ])?,
        PredefinedMenuItem::separator(app)?,
        // 代理组（动态）
        build_proxy_group_submenu(proxies, i18n)?,
        PredefinedMenuItem::separator(app)?,
        // 连接
        Submenu::with_id_and_items(app, "connections", i18n.t("tray.connections"), true, &[
            MenuItem::with_id(app, "close_all", i18n.t("tray.close_all"), true, None)?,
        ])?,
        PredefinedMenuItem::separator(app)?,
        // 更多
        Submenu::with_id_and_items(app, "more", i18n.t("tray.more"), true, &[
            MenuItem::with_id(app, "dev_tools", i18n.t("tray.dev_tools"), true, None)?,
            MenuItem::with_id(app, "move_to_monitor", i18n.t("tray.move_to_monitor"), true, None)?,
            MenuItem::with_id(app, "restart", i18n.t("tray.restart"), true, None)?,
            MenuItem::with_id(app, "force_quit", i18n.t("tray.force_quit"), true, None)?,
        ])?,
        PredefinedMenuItem::separator(app)?,
        MenuItem::with_id(app, "quit", i18n.t("tray.quit"), true, None)?,
    ])?;
    Ok(menu)
}
```

- **菜单事件**：统一在 `tray/events.rs` 处理 → 发送 `tauri::command` 或直接操作 `CoreManager` / `ProxyManager`
- **图标更新**：`speed-update` 由前端计算上传/下载速度 → `invoke('tray_set_icon', {svg_data_url})` → Rust 侧 `tray.set_icon(icon_from_data_url)`

### 3.5 系统代理 / TUN / 回环 (`proxy/`)

| 功能 | 现有实现 | Tauri 实现 |
|------|----------|------------|
| 系统代理 | `EnableLoopback.exe` + 注册表 `HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings` | `winreg` crate 直接写注册表 + `EnableLoopback.exe` sidecar 给 UWP 回环豁免 |
| TUN | `wintun.dll` + `go-tun2socks.exe` 子进程 | `wintun` crate (纯 Rust) + `tokio-tun` / 自建 tun2socks Rust 移植，或保留 `go-tun2socks.exe` sidecar |
| 回环豁免 | `EnableLoopback.exe` + `schtasks.xml` 计划任务 | 同上，保留 sidecar 调用 |

---

## 4. 迁移步骤与里程碑

### Phase 0: 脚手架与基础设施 (Week 1)
- [ ] `cargo tauri init` → 生成 `src-tauri/` + Vue 3 + TS + Vite + Pinia + Vue Router 4 + vue-i18n 9
- [ ] 配置 `tauri.conf.json`：窗口尺寸、图标、标题、sidecar 列表、权限（`shell:allow`, `fs:allow`, `network:allow`, `notification:allow`, `tray:allow`, `autostart:allow`）
- [ ] 建立 `src-tauri/resources/` 目录结构，编写 `build.rs` 复制 `mihomo-win64.exe` 等二进制
- [ ] 配置 Rust 侧 `tracing` + `tauri-plugin-log` → `Data/logs/`
- [ ] CI: GitHub Actions (Windows `windows-latest`) → `cargo tauri build` → 产物上传

### Phase 1: 核心生命周期 + 配置持久化 (Week 2)
- [ ] 实现 `Config` 模型 + `persistence.rs`（读/写 `Data/config.yaml`）
- [ ] 实现 `CoreManager`：启动/停止/重载/版本查询/健康检查
- [ ] 暴露 `core_start`, `core_stop`, `core_restart`, `core_status`, `core_version` 命令
- [ ] 前端 `core.ts` store + `General.vue` 页面（显示版本、状态、启动/停止按钮）
- [ ] **验收**：点击「启动」→ mihomo 进程存活、外部控制器 `127.0.0.1:9090` 可访问、日志输出 `error` 级别

### Phase 2: 配置编辑 + Profile 管理 (Week 3)
- [ ] `config_save` 命令：校验 → 原子写入 → `CoreManager::reload()`
- [ ] 前端 `Settings.vue`：Common/Proxy/TUN/Advanced 标签页，双向绑定 `Config`
- [ ] Profile 列表：`Data/profiles/*.yaml` 扫描、激活、重命名、删除、导入/导出
- [ ] **编辑器**：集成 `@monaco-editor/loader` + `monaco-yaml`（参考现有 `editor.worker.js`）
- [ ] **验收**：修改端口/允许局域网/日志级别 → 保存 → mihomo 热重载生效；切换 Profile → 重载生效

### Phase 3: 代理组、模式、延迟、托盘菜单 (Week 4)
- [ ] `proxies.ts` store：从 `/proxies` + `/providers/proxies` 聚合 → `ProxyGroupInfo[]`
- [ ] 前端 `Proxies.vue`：分组卡片、单选、延迟测试（`/proxies/:name/delay`）
- [ ] 托盘菜单动态构建（`build_tray_menu`）、代理组子菜单、模式单选
- [ ] `tray_set_icon` 接收前端 SVG DataURL → 托盘图标实时速度
- [ ] **验收**：托盘右键 → 代理组/模式/系统代理/TUN 切换即时生效；图标显示上下行速度

### Phase 4: 连接列表、日志、规则、Geo数据 (Week 5)
- [ ] `connections.ts`：轮询 `/connections` → 表格（进程、协议、上下行、规则命中）
- [ ] `logs.ts`：WebSocket `/logs` 或轮询 `/logs` → 过滤、级别、导出
- [ ] `rules.ts`：规则集展示、编辑（只读为主，编辑走 Profile YAML）
- [ ] `geodata_update` 命令：复用 `patch-r8-geodata-recovery.cjs` 逻辑 → Rust 重写（双源、原子替换、回滚）
- [ ] **验收**：连接实时刷新、日志滚动、Geo 更新按钮触发下载→完成→提示重启

### Phase 5: 系统代理/TUN/回环、自启动、便携目录兼容 (Week 6)
- [ ] `system_proxy_enable/disable`：注册表 + `EnableLoopback.exe`
- [ ] `tun_enable/disable`：`wintun` crate 建虚拟网卡 + `go-tun2socks.exe` 转发
- [ ] `autostart_enable/disable`：`tauri-plugin-autostart` + `--clash-edge-autostart` 参数传递
- [ ] **Launcher 兼容**：Rust 启动时读取 `CLASH_EDGE_DATA_DIR` 环境变量 → 覆盖 `app_data_dir()`；若未设置则回退默认
- [ ] **验收**：系统代理开关即时生效、TUN 模式建卡成功、开机自启动静默、便携目录结构不变

### Phase 6: 国际化完善、主题、收尾 (Week 7)
- [ ] 补全 `zh-CN.yaml` / `en-US.yaml` 所有键；CI 校验前后端 key 一致
- [ ] 前端 `ThemeProvider`：`data-theme="dark|light|system"` + CSS 变量
- [ ] 打包优化：`tauri.conf.json` `bundle` 配置、NSIS 可选、图标、版本信息
- [ ] 代码签名准备：`tauri.conf.json` `windows.signCommand` 预留
- [ ] **验收**：全功能回归、中英切换无遗漏、主题跟随系统、体积 < 25 MB、启动 < 2s

---

## 5. 依赖清单 (Cargo.toml 关键项)

```toml
[dependencies]
tauri = { version = "2.0", features = ["tray-icon", "notification", "autostart", "shell", "fs", "dialog", "log", "clipboard-manager", "updater"] }
tauri-plugin-shell = "2.0"
tauri-plugin-fs = "2.0"
tauri-plugin-dialog = "2.0"
tauri-plugin-log = "2.0"
tauri-plugin-notification = "2.0"
tauri-plugin-tray-icon = "2.0"
tauri-plugin-autostart = "2.0"
tauri-plugin-updater = "2.0"
tauri-plugin-clipboard-manager = "2.0"
tauri-plugin-opener = "2.0"

serde = { version = "1.0", features = ["derive"] }
serde_yaml = "0.9"
serde_json = "1.0"
tokio = { version = "1.38", features = ["full"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "fmt", "json"] }
anyhow = "1.0"
thiserror = "1.0"
reqwest = { version = "0.12", features = ["json", "rustls-tls", "proxy", "gzip", "stream"] }
hpagent = "0.3"  # 代理下载
wintun = "0.12"  # TUN 虚拟网卡
winreg = "0.11"  # 注册表
directories = "5.0"  # 跨平台目录
fluent = "0.15"  # 国际化
fluent-templates = "0.11"
once_cell = "1.19"
parking_lot = "0.12"
```

---

## 6. 前端依赖清单 (package.json 关键项)

```json
{
  "dependencies": {
    "vue": "^3.5.41",
    "vue-router": "^4.5.0",
    "pinia": "^2.2.0",
    "vue-i18n": "^10.0.0",
    "@monaco-editor/loader": "^4.0.0",
    "monaco-yaml": "^5.0.0",
    "axios": "^1.7.0",
    "yaml": "^2.5.0",
    "sortablejs": "^1.15.0",
    "vuedraggable": "^4.1.0",
    "element-plus": "^2.8.0"
  },
  "devDependencies": {
    "@tauri-apps/cli": "^2.0.0",
    "@vitejs/plugin-vue": "^5.1.0",
    "typescript": "^5.5.0",
    "vite": "^5.4.0",
    "sass": "^1.77.0",
    "eslint": "^9.9.0",
    "prettier": "^3.3.0"
  }
}
```

> UI 库选 **Element Plus**（成熟、TS 支持好、体积可 tree-shaking），或按团队偏好换 Naive UI / Ant Design Vue。不新增组件，仅复用现有视觉规范。

---

## 7. 兼容性清单（必须保留的行为）

| 行为 | 现有实现位置 | Tauri 迁移位置 |
|------|--------------|----------------|
| 便携目录 `Data/` 隔离 | C# Launcher Junction | `src-tauri/src/util/paths.rs` 读 `CLASH_EDGE_DATA_DIR` |
| 静默自启动 `--clash-edge-autostart` | `patch-r8-quiet-start.cjs` | `tauri-plugin-autostart` + `args: ["--clash-edge-autostart"]` |
| 外部控制器强制 `127.0.0.1` + 随机密钥 | `profile-preprocessor.cjs::normalizeController` | `core/config.rs` 生成配置时固定 |
| 允许局域网仅显式开启 | `allow-lan: false` 默认 | `Config.allow_lan` 默认 `false` |
| 日志级别默认 `error` | `log-level: error` | `Config.log_level = LogLevel::Error` |
| Geo 数据手动更新、原子替换、回滚 | `patch-r8-geodata-recovery.cjs` | `geodata/updater.rs` |
| 订阅规则保留、内置规则前置、MATCH 兜底 | `profile-preprocessor.cjs::buildPreset` | `core/config.rs` 渲染模板时保持相同顺序 |
| 托盘图标速度显示（Win 专用 HTML overlay） | `main.js speed-update` handler | 前端计算 → `invoke('tray_set_icon', {svg})` |
| 窗口隐藏而非关闭（点击关闭按钮） | `main.js window-control hide` | `tauri.conf.json` `disableClose: true` + `window.onCloseRequested` → `hide()` |
| 开发者工具切换 | `webContent toggleDevTools` | `app_handle.webview_windows().get("main").unwrap().open_devtools()` |
| 证书错误拦截并询问信任 | `main.js certificate-error` | `tauri.conf.json` `dangerousDisableAssetVerification: false` + 自定义协议处理 |

---

## 8. 验收标准（Definition of Done）

1. **功能对等**：R8.3 所有可观测行为在 Tauri 版 1:1 复现（通过手工测试清单核对）
2. **体积**：安装包/便携包 **< 25 MB**（含 mihomo、wintun、go-tun2socks、EnableLoopback）
3. **启动**：冷启动到主窗口显示 **< 2 秒**（Windows WebView2 已预装）
4. **内存**：空闲驻留 **< 80 MB**（Electron 通常 150–200 MB）
5. **安全**：`cargo audit` 0 高危；前端无 Node 集成；IPC 入口全部类型化
6. **国际化**：中/英零硬编码；CI 强制 key 对齐
7. **便携性**：解压即用，无需安装，数据目录自包含，Launcher 行为不变
8. **签名就绪**：`tauri.conf.json` 预留 `signCommand`，证书配入即可出包

---

## 9. 风险与对策

| 风险 | 影响 | 对策 |
|------|------|------|
| **wintun Rust crate 不稳定 / API 变动** | TUN 模式无法工作 | 兜底：保留 `go-tun2socks.exe` + `wintun.dll` sidecar，Rust 仅做进程托管 |
| **Monaco Editor 体积大** | 包体积超标 | 动态 `import()` 按需加载；仅 Profile 编辑页加载；`vite.config.ts` `manualChunks` 分离 |
| **系统代理注册表权限** | 非管理员写 HKCU 可能失败 | 仅操作 `HKCU`（用户级），不碰 `HKLM`；失败回退提示 |
| **便携目录 Junction 权限** | 无权限创建联接 | Launcher 保留 Junction 创建职责；Rust 仅读取 `CLASH_EDGE_DATA_DIR` |
| **前端重写工作量被低估** | 进度延期 | Phase 1–2 先跑通核心链路，UI 组件复用 Element Plus 标准件，仅业务逻辑自写 |
| **Rust 学习曲线** | 团队不熟悉 | 核心模块由熟手把脉，边界清晰（`#[tauri::command]` 即契约） |

---

## 10. 后续：发布流水线

```yaml
# .github/workflows/release.yml 片段
jobs:
  build:
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: actions/setup-node@v4
        with: { node-version: '22', cache: 'npm' }
      - run: npm ci
      - run: npm run tauri build
      - uses: actions/upload-artifact@v4
        with: { name: clash-edge-portable, path: src-tauri/target/release/bundle/nsis/*.exe }
```

---

**文档版本**：v0.1（草案）  
**下一步**：请审阅确认范围与优先级，随后我进入 Phase 0 脚手架搭建。