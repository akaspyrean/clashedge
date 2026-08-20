# ClashEdge

> Lightweight Clash client for Windows — 基于 Tauri 2 + Mihomo 内核，便携免安装。

ClashEdge 是一个面向 Windows 的轻量 Clash 图形客户端。基于 **Tauri 2（Rust 后端）+ Vue 3（前端）+ Mihomo 内核**构建，定位「轻量、便携、状态严格一致」：界面状态 = 应用状态 = Mihomo 实际状态 = Windows 实际状态，任何功能变更都经过「校验 → 持久化 → 重生成运行时配置 → 实时下发 → 失败回滚」链路。

## 功能特性

- **便携免安装**：解压即用，无需安装。`App` / `Data` / `Other` 三分离——整体复制、移动、改名、换盘、换电脑后仍可直接运行，不依赖 CWD、盘符、用户名或绝对路径。
- **托盘驻留**：关闭窗口即最小化到系统托盘（首次提示）。托盘图标随系统代理状态变色（开=绿 / 关=蓝），右键菜单可快捷切换代理模式等。
- **Mihomo 内核**：内置 sidecar 内核，开箱即用；单实例保护，二次启动自动聚焦既有窗口。
- **内置代理结构**：固定 6 组骨架（`GLOBAL` + 扶梯出行 / 人工智能 / 影音视听 / 人工优选 / 自动优选）+ 内置规则链 + 4 个内置规则集（direct / proxy / media / ai），订阅只需提供节点，节点自动注入叶子组，规则模式切换（rule / global / direct）实时生效。
- **订阅管理**：URL 订阅导入、更新（拉取失败不覆盖原配置，激活中的订阅热重载生效）、新建/导入/导出/重命名/原始编辑配置，激活配置实时下发。
- **系统代理**：一键开关，直写注册表 + WinINet 通知；应用退出时按快照还原/清除系统代理。
- **TUN 模式**：基于 `go-tun2socks` + `wintun` 的透明代理支持。
- **延迟测速**：手动 / 分组测速，结果分批刷新，不卡界面。
- **连接管理**：实时连接列表与流量查看。
- **Geodata 管理**：geoip / geosite / mmdb 下载与更新（支持自定义源、下载大小上限保护）。
- **日志与排障**：内置日志查看；日志自动轮转防止磁盘占满；内核端口冲突等错误如实上报（顶部横幅），绝不假装运行中。
- **安全加固**：控制器密钥首次运行随机生成并持久化（配置重置/导入/更新时自动轮换）；系统代理开启前密钥兜底；SSRF 防护（订阅/Geodata/测速 URL 统一校验：禁段 IP、DNS 反查、重定向逐跳校验、IPv4-mapped 地址防绕过）；WebView 导航锁定 + 收紧 CSP + 最小化权限；provider path 强制相对路径防越权读取。
- **界面**：深浅色主题（设计系统 token，跟随系统/手动切换）、中英双语（i18n）、6 大导航页（概览 / 代理 / 连接 / 配置 / 日志 / 设置）。

## 技术栈

| 端 | 技术 |
| --- | --- |
| 应用外壳 | Tauri 2（Rust，`tauri::command` + sidecar 进程管理） |
| 前端 | Vue 3.5 + Pinia + Vue Router 4 + Vite 6 + Element Plus + vue-i18n |
| 内核 | Mihomo v1.19.20（`clash-edge-core.exe` sidecar） |
| TUN | go-tun2socks + wintun |
| 系统代理 | winreg 直写注册表 + WinINet 通知刷新 |

## 快速开始

1. 从 Releases 下载 `ClashEdge-portable-<version>-win64.zip` 并校验 `.sha256`。
2. 解压到任意目录（建议非系统盘，路径可含空格/中文）。
3. 运行根目录 `ClashEdge.exe`——静默驻留托盘，初次运行自动生成随机控制器密钥并初始化用户数据。
4. 在「配置」页导入或新建配置，再到「概览」页启动核心。

> 若顶部横幅提示端口被占用，通常是旧版 Clash 客户端仍在运行——先关闭旧客户端再启动。

## 目录结构

```
.
├── tauri-scaffold/        # Tauri 应用源码（前端 src/ + Rust 后端 src-tauri/src/）
├── portable-template/     # 便携版模板：内核/侧车二进制、默认数据、规则文件
├── tools/                 # 构建/打包脚本（build-portable.ps1）、图标
├── docs/                  # 文档：发布报告、安全审核报告、交接日志、历史规划
├── release/               # 打包产物（便携目录 + zip + SHA256，不入库）
└── .claude/               # 项目级协作规则
```

## 环境要求

- Windows 10/11（依赖系统 WebView2 运行时，通常已预装）
- Rust stable（`rustup`）+ Cargo
- Node.js 18+（含 `npm`）
- Tauri 2 CLI（通过 `npm` 调用）

## 构建

```powershell
# 前端类型检查 + 生产构建
cd tauri-scaffold
npm install
npm run build

# Rust 单元测试
cd src-tauri
cargo test

# 发布构建（产物：src-tauri/target/release/ClashEdge.exe）
cd ..
npm run tauri -- build --no-bundle
```

## 打包

```powershell
# 组装便携目录 + 生成 zip 与 SHA256（需先完成发布构建；-Build 可一并执行构建）
.\tools\build-portable.ps1 [-Build]
```

产物输出到 `release/`：

- `release/portable-out/` — 便携目录（根启动器 `ClashEdge.exe` + `App/` + `Data/` + `Other/`）
- `release/ClashEdge-portable-<version>-win64.zip` — 单文件分发包
- `release/ClashEdge-portable-<version>-win64.zip.sha256` — 校验文件

## 便携结构与自维护

- **App/** — 程序文件（内层应用 + sidecar 内核与辅助程序）
- **Data/** — 用户数据：`config.yaml`、`profiles/`、`logs/`、`geodata/`、`rules/`
- **Other/** — 发行辅助文件（`Help/README.md`）
- 便携模式自动检测，整体复制/移动后程序自动修复注册表自启动路径指向当前位置。
- 便携模式日志写入 `Data/logs/` 随包迁移；安装版写入 OS 默认日志目录。

## 已知环境问题

- 若旧版 Clash.F.Win（Electron 版）仍在运行，会占用 `127.0.0.1:7890` 与 DNS `9053`，导致本应用内核绑定失败——新版本会如实报错（顶部横幅）而非假装运行中，请先关闭旧版再启动。
- 订阅类规则集（direct/proxy/media/ai）首次加载使用内置本地文件；需要网络时请配置可用订阅。

## 文档

- [`docs/RELEASE_REPORT.md`](docs/RELEASE_REPORT.md) — 0.8.5 稳定版发布报告（含上线前安全审核与最终修订）
- [`docs/PRE-LAUNCH-AUDIT-REPORT.md`](docs/PRE-LAUNCH-AUDIT-REPORT.md) — 上线前安全/BUG 审核报告
- [`docs/HANDOVER.md`](docs/HANDOVER.md) — 工作交接日志（当前状态、修订内容、构建约定）
- [`docs/REFACTOR-PLAN-TAURI.md`](docs/REFACTOR-PLAN-TAURI.md) — Tauri 2 重构实施规划
- [`docs/REPAIR_PLAN.md`](docs/REPAIR_PLAN.md) — 内部修复方案（历史）
- [`docs/archive/R8.3-REVISION-NOTES.md`](docs/archive/R8.3-REVISION-NOTES.md) — R8.3 修订说明（Electron 时代，归档）