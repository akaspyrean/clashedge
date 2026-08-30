# ClashEdge 架构

> 本文只描述**当前**架构。历史决策、修订记录见 Git history 与 Release notes。

## 定位与原则

ClashEdge 是 **Mihomo 之上的跨平台控制层**：订阅提供节点，ClashEdge 负责分流与策略，
把 80% 高频动作做到极其简单。三个原则贯穿所有决策：

- **Mihomo 能做的，不重做。** 分流、TUN、DNS、规则匹配都是内核的能力。
- **操作系统能做的，不抽象。** 系统代理、自启动直接走 Windows 原生机制。
- **GitHub 能做的，不自己造治理系统。** 发布治理交给 Rulesets / Actions，
  仓库只保留产品安全必需的校验（SHA256、清单签名、Updater 验签）。

## Non-goals

ClashEdge ≠ Mihomo fork、≠ Sub-Store、≠ 规则编辑 IDE、≠ 云同步平台、≠ 插件平台。
高级用户直接编辑 Mihomo YAML；不为 Android 提前抽象"跨平台公共框架"——
Windows 是 Rust/Tauri，Android 是 Kotlin/Compose/VpnService，允许重复少量业务代码。

## 仓库结构

```text
assets.lock.json        # 第三方资产（内核/驱动/规则集）的版本/URL/SHA256 锁定
apps/
  windows/              # 正式产品：Tauri 2（Rust 后端 + Vue 3 前端）
  android/              # 冻结实验：无真实内核，进入发布链路前须过 apps/android/README 的前置项
packaging/windows/      # 便携包模板（DefaultData、launcher 源码、Other/Help）
scripts/
  assets/prepare.ps1    # 下载 → 校验 SHA256 → 缓存 → stage 第三方资产
  ci/quality.ps1        # 唯一质量门（CI 与 Release 共用）
  windows/              # build-portable.ps1、scan-portable-paths.ps1
  release/make-update-manifest.py
tests/fixtures/         # 跨语言共享的测试夹具
build/assets/           # prepare.ps1 的缓存与 staging（gitignored）
```

## Windows 端

前端 Vue 3 + Pinia + Router + vue-i18n + Element Plus；后端 Rust + Tokio + Tauri 2。
依赖刻意克制：不重写 UI 组件库、不引入微前端、不加跨平台运行时。

### 核心不变量

> **界面状态 = 应用状态 = Mihomo 实际状态 = Windows 实际状态。**

任何功能状态变更都必须走同一条链路：

```text
校验 → 持久化 → 重生成 runtime-config → 实时下发给 mihomo → 失败回滚 → 通知 UI/托盘刷新
```

这条链路由 **AppController** 从机制上强制维持：事务串行锁在每个 controller 方法
内部获取并持有到事务结束，command / 托盘等调用方无法绕过，也无法忘记加锁。

### 配置双层模型

- `Data/config.yaml` 是**应用配置**（AppConfig）：locale、geodata 模式、激活的 profile 等。
- mihomo 加载的是 `Data/runtime-config.yaml`：由 `core::config::build_runtime_config`
  把 AppConfig 与激活 Profile 合成。**订阅只提供节点**——应用始终使用内置 6 组骨架
  （GLOBAL + 扶梯出行/人工智能/影音视听/人工优选/自动优选）与内置规则链，把订阅节点名
  强制注入叶子组；订阅自带的 proxy-groups/rules 一律不整组采用。

内置规则链（顺序固定）：
`GEOSITE,private → RULE-SET,direct → RULE-SET,ad → GEOSITE,category-ads-all →
RULE-SET,ai → RULE-SET,media → RULE-SET,proxy → GEOSITE,cn → GEOIP,CN → MATCH`。

### 后端模块

```text
src-tauri/src/
  main.rs           # Tauri 装配：AppState、command 注册、托盘、事件
  commands/         # Tauri command 层：参数 → AppController → Result（不放业务状态）
    profiles/       #   mod.rs 命令层 + validate / files / subscription 逻辑模块
  core/
    app_controller.rs  # AppController：唯一修改边界，事务串行锁 + 全链路内聚
    manager.rs         # CoreManager：struct、状态、REST 透传（门面）
    lifecycle.rs       # 进程生命周期：start/stop/restart/reload、runtime-config 落盘
    supervisor.rs      # watcher、自动重启、崩溃熔断、PID 缓存、绑定冲突检测
    config.rs          # runtime-config 合成（AppConfig + Profile）
    controller.rs      # mihomo 外部控制器 REST 客户端（无进程状态）
    runtime.rs         # 事务链实现（*_locked）与运行时状态投影
    health.rs          # 健康检查
  config/           # AppConfig 的 model / persistence / migration
  proxy/            # system_proxy（Windows 注册表）、journal（状态事务日志）
  geodata/          # GeoIP/GeoSite 下载源与更新
  tray/             # 托盘图标与菜单（随系统代理状态变色）
  update/           # 更新检查与便携包清单验签
  util/             # fetch/（受限 HTTP 客户端：guards=SSRF 防护、client=下载机制）、
                    # paths（便携检测）、atomic、autostart、elevation、normalizer
  i18n/             # 后端文案加载
```

### 便携布局（三分离）

```text
ClashEdge.exe            # C# launcher：设 CLASH_EDGE_DATA_DIR，拉起内层应用
App/ClashEdge/           # Tauri 应用本体 + sidecar/（mihomo-win64.exe、wintun.dll）
App/DefaultData/         # 出厂默认数据（GeoIP/GeoSite/Country.mmdb/config.yaml）
Data/                    # 用户数据（config.yaml、runtime-config.yaml、profiles、logs、rules）
Other/Help/              # 附属文档
```

便携判定：`App/portable.dat` **或** `App/clash-edge-core.exe` 存在（改名/换盘符可自愈）。
便携模式下 mihomo 固定解析 `<exe_dir>/App/clash-edge-core.exe`，无 %APPDATA% 静默回退。

## 发布与更新

```text
push v* tag → quality.ps1（fmt/clippy/test/audit/前端测试/build）
           → build：Tauri --no-bundle → prepare.ps1 取内核 → build-portable.ps1 组包
           → ZIP（稳定名 ClashEdge-portable-win64.zip）+ SHA256
           → manifest + minisign 签名 → dry-run 校验 → attest → 发布
```

- 触发只有 `push: tags: v*`；tag 不可变由 GitHub Rulesets 保证（仓库设置，非脚本）。
- manifest 强制签名（`TAURI_SIGNING_PRIVATE_KEY`），公钥编译期注入客户端（`update/mod.rs`）。
- Updater 按稳定 ZIP 名下载，验 SHA256 + minisign 签名。
- 第三方资产（mihomo、wintun.dll、内置规则集、geodata）不进 Git：版本/URL/SHA256
  锁在 `assets.lock.json`，由 `scripts/assets/prepare.ps1` 物化到 `build/assets/staging/`。
  规则集与 geodata 由 `akaspyrean/external` 仓库发布（geodata 由其定时同步 Action
  镜像 MetaCubeX/meta-rules-dat），均固定到具体 commit（commit-sha raw URL 永久不可变），
  升级 = 改 lock 的 commit + 各文件 sha256 后提交，hash 由脚本逐字节校验，
  绝不为未知二进制生成可信哈希。

## 测试与质量

- `scripts/ci/quality.ps1` 是唯一质量门：cargo fmt/clippy/test、cargo audit、
  npm audit、Vitest、前端 build；CI（push/PR）与 Release（tag）调用同一份。
- Launcher 故障注入测试（`ClashEdge.exe --test-recovery`）纳入 Release：
  5 个 kill 点（pending/verified/swapping/committed/nojournal）必须全部恢复成功。
- 需要人工观察的场景不进仓库门禁，靠长期实际使用发现。

## 已知债务（有意推迟，非遗忘）

- 上一轮的两项主要债务（AppController 收拢、manager/fetch/profiles 拆分）已完成。
  `AppState` 现在只剩辅助成员（tray、log_stream、core_pid_cache、verified_update），
  全部修改路径必须经 `AppController`。新增修改状态的功能时，一律加 controller 方法，
  不得在 command / tray 层直接操作 ConfigManager + CoreManager。
