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
tests/                  # 跨语言共享的测试夹具（Rust 测试使用内联 YAML，无需外部文件）
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
  设置页暴露的每个开关都必须有后端消费方（不允许"只翻转一个布尔值"的死开关）。
- mihomo 加载的是 `Data/runtime-config.yaml`：由 `core::config::build_runtime_config`
  把 AppConfig 与激活 Profile 合成。**订阅只提供节点**——应用始终使用内置 6 组骨架
  （GLOBAL + 扶梯出行/人工智能/影音视听/人工优选/自动优选）与内置规则链，把订阅节点名
  强制注入叶子组；订阅自带的 proxy-groups/rules 一律不整组采用。

内置规则集一律是本地 `type: file`（`Data/rules/*.yaml`）：刷新由 `geodata::rules` 的**签名清单
更新器**完成（minisign 验签 + 单调版本 + 路径/URL 白名单 + SHA256 + 内容校验 + 事务替换），
不再让 mihomo 从 `main` 分支浮动拉取。GeoIP/GeoSite 写入 **Data 根目录**（mihomo 只读那里）。

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
    lifecycle.rs       # 进程生命周期：start/stop/restart/reload（runtime-config 哈希未变则跳过热重载）
    spawn.rs           # 首次启动与崩溃重启共用：写 runtime-config、日志追加+轮转、core-session.json、
                       #   孤儿 mihomo 回收、端口预检（mixed/DNS 占用报占用者；控制器端口被占自动改选）
    supervisor.rs      # watcher、自动重启、崩溃熔断、PID 缓存、绑定冲突检测
    config.rs          # runtime-config 合成（AppConfig + Profile）
    controller.rs      # mihomo 外部控制器 REST 客户端（无进程状态）
    runtime.rs         # 事务链实现（*_locked）与运行时状态投影
    health.rs          # 健康检查
  config/           # AppConfig 的 model / persistence / migration
  proxy/            # system_proxy（Windows 注册表）、journal（状态事务日志）
  geodata/          # download.rs（受限下载/事务替换）、rules.rs（签名规则清单更新器）、
                    # updater.rs（GeoIP/GeoSite 写入 Data 根目录并重载）、sources.rs
  tray/             # 托盘图标与菜单（随系统代理状态变色）
  update/           # 更新检查、清单验签、暂存（含已验签清单）、--verify-staged/--verify-signature CLI
  util/             # fetch/（受限 HTTP 客户端：guards=SSRF 防护（感知 TUN+fake-ip）、client=下载机制）、
                    # paths（便携检测）、atomic、autostart、elevation、normalizer、
                    # process（持句柄终止/端口占用者）、temp（RAII 临时文件）、uri_list（分享链接/Base64 订阅）
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

进程与端口：mihomo 的 PID+映像路径落盘到 `Data/core-session.json`；应用被强杀/崩溃后，下次启动先回收
本应用遗留的孤儿内核（映像路径校验 + 持句柄终止），再做端口预检。终止进程统一走
`util::process::terminate_owned`（不使用 `taskkill`）。

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
- Updater 按稳定 ZIP 名下载，验 SHA256 + minisign 签名；已验签清单与签名随 ZIP 一起暂存，启动器在
  应用更新前调用当前已安装的内层程序 `--verify-staged` 复验（不信任 `pending.json` 自带哈希），
  应用仍在运行时启动器跳过更新并保留暂存区；应用内"重启并安装"用 `--wait-pid` 等旧进程退出。
- 发布流水线按权限拆分：`build`（只读令牌、无签名私钥，运行第三方构建代码）→ `publish`（写权限+私钥，
  `npm ci --ignore-scripts`），发布前用刚构建的客户端自带验签器（`--verify-signature`）验证签名。
- 第三方资产（mihomo、wintun.dll、内置规则集、geodata）不进 Git：版本/URL/SHA256
  锁在 `assets.lock.json`，由 `scripts/assets/prepare.ps1` 物化到 `build/assets/staging/`。
  规则集与 geodata 由 `akaspyrean/external` 仓库发布（geodata 由其定时同步 Action
  镜像 MetaCubeX/meta-rules-dat），均固定到具体 commit（commit-sha raw URL 永久不可变），
  升级 = 改 lock 的 commit + 各文件 sha256 后提交，hash 由脚本逐字节校验，
  绝不为未知二进制生成可信哈希。

## 测试与质量

- `scripts/ci/quality.ps1` 是唯一质量门：cargo fmt/clippy/test、cargo audit、cargo deny
  （licenses/bans/sources，见 `src-tauri/deny.toml`）、npm audit、Vitest、前端 build；CI（push/PR）与 Release（tag）调用同一份。
- Launcher 故障注入测试（`ClashEdge.exe --test-recovery`）纳入 Release：
  5 个 kill 点（pending/verified/swapping/committed/nojournal）必须全部恢复成功。
- 需要人工观察的场景不进仓库门禁，靠长期实际使用发现。

## 已知债务（有意推迟，非遗忘）

- `serde_yaml` 已归档（解析不可信订阅）：迁移需要统一的 YAML 封装层，单独排期。
- 控制器仍走回环 TCP（已做：每次启动端口冲突自动改选、随机 secret、`no_proxy`）；Windows 命名管道
  需要自定义 hyper connector，后续评估。
- 订阅 URL（含 token）仍明文保存在 `profiles/*.yaml` 头注释：DPAPI 加密会破坏"整体迁移到另一台机器"
  这一便携核心卖点，暂缓；需要时应提供"导出时脱敏"。
- Windows ARM64：`assets.lock.json` 仅 amd64 内核与 wintun。
- Android 冻结，见 `apps/android/README.md`（订阅导入仍是只留 name/type/server 的骨架）。
- 新增修改状态的功能时，一律加 AppController 方法。**整个 `src/` 里只有 `app_controller.rs`、
  `runtime.rs`（仅在控制器持锁时被调用）、`persistence.rs` 可以直接 `set_config(..)`**，
  由 `main.rs::architecture_guards` 测试强制；崩溃自愈 / 启动恢复这类内部路径用
  `AppController::disable_system_proxy_intent` 或 `set_config_unless_degraded`。
- 降级模式（config.yaml 损坏）：任何写盘入口都被拒绝，内部路径只改内存；窗口顶部横幅的
  「覆盖并继续」（`confirm_overwrite_corrupt_config`）是唯一的解除方式。
- 控制器端口被占用时的改选地址只存在于 `CoreManager::controller_override`（会话级），绝不写回
  共享配置（共享配置会被任意设置保存落盘）。
- 锁序固定为 `AppController.tx → CoreManager.lifecycle`；持 `lifecycle` 时不得再取 `tx`
  （看门狗在系统代理恢复前释放 lifecycle 就是为此）。
- TUN + fake-ip 下本地 DNS 结果不可信：SSRF 守卫改用 DoH（`util/fetch/doh.rs`）取真实地址，
  取不到即拒绝（fail-closed）。
- 发布权限拆分：`build`（无私钥、无 rust-cache）→ `publish`（`release` Environment；签名
  证书与 minisign 私钥只在此处，Authenticode 签名与 manifest 生成也在此处）。
