# ClashEdge 工作交接日志（HANDOVER）

> 最近更新：2026-08-23　版本：1.0.0　架构：Tauri 2（Rust 后端 + Vue 3 前端）+ Mihomo v1.19.20

## 1. 项目概览

| 项 | 值 |
| --- | --- |
| 产品名 | ClashEdge（原 Clash.F.Win 统一更名） |
| 应用标识 | `com.clashedge.portable` |
| 版本 | 1.0.0 |
| 内核 | clash-edge-core.exe v1.19.20（sidecar） |
| 前端 | Vue 3.5 + Pinia + Vue Router 4 + Vite 6 + Element Plus + vue-i18n |
| 后端 | Rust（`tauri::command` + sidecar 进程管理；winreg 直写系统代理） |
| 形态 | Windows 便携目录（根启动器 `ClashEdge.exe` + `App/` + `Data/`） |

核心目标（贯穿所有修订）：**界面状态 = 应用状态 = Mihomo 实际状态 = Windows 实际状态**。
任何功能状态变更一律经过「校验 → 持久化 → 重生成运行时配置 → 实时下发给 mihomo → 失败回滚 → 通知并刷新」链路。

## 2. 目录结构

```
.
├── tauri-scaffold/        # Tauri 应用源码
│   ├── src/               #   前端：views / stores / api / i18n / router
│   └── src-tauri/src/     #   Rust 后端：commands / config / core / proxy / geodata / tray / util
├── portable-template/     # 便携模板：侧车二进制 + 默认数据 + 规则集（供打包拷贝）
├── tools/                 # 构建/打包脚本与图标
│   ├── build-portable.ps1 #   打包入口（组装便携目录 + zip + SHA256；含前置/后置校验）
│   ├── ClashEdge.ico      #   启动器图标
│   └── ClashEdge.Launcher.R8.2.cs  # C# 根启动器源码（build-portable.ps1 每次打包时用 csc.exe 编译为便携包根目录的 ClashEdge.exe）
├── docs/                  # 文档
│   ├── HANDOVER.md        #   （本文档）
│   ├── RELEASE_REPORT.md  #   0.8.5 稳定版发布报告
│   ├── REFACTOR-PLAN-TAURI.md  # Tauri 2 重构实施规划
│   ├── REPAIR_PLAN.md     #   内部修复方案（历史）
│   └── archive/           #   归档：R8.3 修订说明（Electron 时代，已不适用）
├── release/               # 打包产物（不入库，gitignore）
│   ├── portable-out/      #   便携目录
│   └── ClashEdge-portable-<ver>-win64.zip(.sha256)
├── README.md              # 项目总览与构建/打包指南
└── .gitignore
```

## 3. 构建与打包

```powershell
# 前端类型检查 + 生产构建
cd tauri-scaffold; npm install; npm run build

# Rust 单元测试
cd tauri-scaffold/src-tauri; cargo test

# 发布构建（一次，产物 target/release/ClashEdge.exe）
cd tauri-scaffold; npm run tauri -- build --no-bundle

# 打包（产物输出到 release/）
.\tools\build-portable.ps1
```

打包前约定：`build-portable.ps1` 会先删除 `release/portable-out/`；**运行中的旧测试实例必须先停**（PID + ExecutablePath 双重校验，仅限测试目录下的进程）。

> 脚本注意事项：release 目录变量名为 **`$releaseDir`**。不要改名为 `$rel`——脚本文件清单循环（`Get-ChildItem ... | ForEach-Object`）已占用 `$rel` 作相对路径循环变量，同名会导致 zip 目标路径错乱（曾因此打包失败，已修复）。

## 4. 修订记录

### 1.0.0（当前版本，2026-08-23）

- **核心启动修复（零节点路径）**：mihomo v1.19.20 拒绝 proxies 为空的代理组；零订阅时自动优选组整体不生成（保持"url-test 只含真实节点"约束），其余组引用同步剔除，人工优选补 DIRECT 兜底——无订阅也能启动核心并全量直连。测试 `build_runtime_config_zero_nodes_drops_auto_group_keeps_manual_direct` / `..._subscription_restores_full_group_structure` 覆盖。
- **设计系统 v1.0 固化**：新增 `docs/DESIGN_SYSTEM.md`（Quiet Power 品牌性格、Flyme 3 + Apple 融合原则、WCAG AA 对比度实测表）；`styles.css` token 化补齐间距/动效 token，深色色板按 Apple HIG 海拔分层重调（accent `#77A7FF`→`#4E8FFF`）。
- **UI 全局统一**：页面标题与导航名一致（代理/配置/连接，i18n zh+en）；工具栏统一左对齐序列；状态指示统一「小圆点+文字」语言（概览/日志页）；danger 按钮全应用 plain 化；配置卡按钮语义收敛（激活=primary plain，更新/重命名=默认，删除=danger plain）。
- **修复卡片头贴边 bug**：DashboardView 双值 `--el-card-padding: 20px 24px` 导致 EP 的 `calc()` 失效、头部内边距归零；改单值 token 并在全局显式定死头部内边距兜底。
- **默认窗口 832×554**（初版 850×603 → 1080×720 → 756×504 → ×1.1），窄窗断点同步收紧至 749px——首启即展示带文字侧栏；最小尺寸 560×400。
- **P1-m 确认已修**：设置页导入走后端 `read_import_file` 命令，前端零 fs 依赖（RELEASE-GATE 阻断项解除）。
- 版本号三处一致：tauri.conf.json / package.json / Cargo.toml = 1.0.0。

### 0.8.7（历史）

- **从 Delta 项目移植工程化基建**：CI best practices（ci.yml、workflow_dispatch、版本号校验）、release workflow（幂等 draft、`--clobber` 覆盖、并发锁、过期 draft 清理）、路径泄露扫描与稳定文件名下载。
- **自动更新基础设施移植（半成品）**：注册 `tauri-plugin-updater` 并在 `tauri.conf.json` 配置 endpoints，但 pubkey 为空、无任何检查更新的前后端逻辑——**不可用**，Phase 3 将重做 Portable Updater 并移除该半成品（见 `docs/AUDIT-0.8.7.md`）。
- Phase 0 基线审计完成，产出 `docs/AUDIT-0.8.7.md` 与 `docs/RELEASE-GATE.md`。

### 0.8.5（历史）

1. **代理组结构按规范重排（6 组，含 GLOBAL）**：主组「扶梯出行」，下辖「人工智能」「影音视听」两个场景组，每组可选「人工优选 / 自动优选」；
   - 顶部另设 GLOBAL：`type: select`，成员 `[DIRECT, REJECT, 人工优选, 自动优选]`——全局模式专用组（`mode: global` 时所有流量走它），仅存在于配置；代理页面按模式联动显示（rule 隐藏 GLOBAL / global 独占显示 / direct 显示直连提示，前端 `ProxiesView` 实现），托盘保留以便 global 模式切换目标。
   - 扶梯出行 / 人工智能 / 影音视听：`type: select`，子项 `[人工优选, 自动优选]`
   - 人工优选：`type: select`（含全部节点，手动选择）
   - 自动优选：`type: url-test`（url `https://cp.cloudflare.com/generate_204`，interval 300，tolerance 100）
   - 规则链：`GEOSITE,private,DIRECT` → `RULE-SET,direct,DIRECT` → `RULE-SET,ad,REJECT` → `GEOSITE,category-ads-all,REJECT` → `RULE-SET,ai,人工智能` → `RULE-SET,media,影音视听` → `RULE-SET,proxy,扶梯出行` → `GEOSITE,cn,DIRECT` → `GEOIP,CN,DIRECT` → `MATCH,扶梯出行`
2. **代理端口绑定失败不再假成功**：`core/manager.rs` 新增绑定冲突检测（读取 `Data/logs/mihomo-stdout.log`，匹配 `level=error` + `bind`），检测到即停止内核、状态置 `Error`、前端顶部红条如实提示。单测 `test_parse_bind_error_detects_port_conflict` 覆盖。
3. **配置文件页默认「订阅」**：工具栏「订阅」为主按钮，置于「新建配置」之前。
4. **ai 规则集修复**：`portable-template/App/DefaultData/rules/ai.yaml` 移除 `ip-asn,20473,no-resolve` 行——mihomo `behavior: classical` 遇到 `ip-asn` 会整文件失败（0 规则）。移除后实测加载 **81 条**规则。
5. **重命名订阅名修复**：`rename_profile` 是唯一带多词 snake_case 参数（old_name/new_name）的命令；Tauri 2 的 `invoke` 按 **camelCase** 匹配前端 key，`api/profiles.ts` 误传 `old_name/new_name` 导致 `missing required key oldName`。已改为 `{ oldName, newName }`（该约定仅影响多词参数，其余命令参数均为单词不受影响）。
6. **托盘图标随系统代理状态变色**：`tray/builder.rs` 新增 `build_tray_icon(config)`，运行时解码内置 `icons/32x32.png` 并按 RGBA 重绘——系统代理**开 → 绿 #15803D**（活跃态）、**关 → 蓝 #2A62CC**（闲置态）。按目标色着色：亮度因子 `f = 0.6 + 0.4·l/255`（映射到 [0.6, 1.0]），最高亮像素即目标色本身、暗部为目标的 60%，色相与目标色严格一致，亮度区间保证浅色/深色任务栏均清晰可辨。`build_tray` 初始图标按真实配置着色，`update_tray_menu` 每次刷新（`refresh_tray`）都 `set_icon` 同步。`Cargo.toml` 直接声明 `image = 0.25`（本就是依赖树内 crate，零新增下载）。
7. **叶子组不含 DIRECT**：`core/config.rs` 注入订阅节点时——**人工优选**（手动选择）只注入真实节点、**不含 DIRECT**；**自动优选**（url-test）保留 DIRECT 作兜底（全部节点失败时直连）。前端 `ProxiesView.visibleProxies` 对自动优选过滤 `DIRECT`（人工优选本无 DIRECT，过滤为 no-op，保留兼容）；`proxyStore.testGroupProxies` 同样过滤 DIRECT。
8. **规则模式组固定排序**：`ProxiesView` 显式排序「扶梯出行 → 人工智能 → 影音视听 → 人工优选 → 自动优选」——mihomo `/proxies` 按 Go map 返回无序，必须前端排序。
9. **载入配置强制注入节点**：`core/config.rs` 删除 `self_contained`（订阅自带完整 proxies+proxy-groups+rules 时整组采用）分支。应用始终采用内置 6 组骨架 + 内置规则链，订阅**只提供节点**，节点名强制注入「人工优选/自动优选」叶子组——即使订阅自带 proxy-groups/rules 也不再整组采用（其规则引用的组在应用中不存在，整组采用会导致叶子组拿不到节点）。测试改写为 `build_runtime_config_always_injects_subscription_nodes_into_leaf_groups`。
10. **侧栏冗余品牌名去除**：`App.vue` 移除侧栏顶部 `.app-logo`（logo 圆点 + "ClashEdge" 文本）——自绘标题栏已有品牌名，侧栏不再重复；同步清理 `styles.css` 死代码 `.app-logo`/`.logo-dot`。侧栏顶部直接从导航菜单开始，底部版本号保留。
11. **概览页系统代理开关**：`DashboardView.vue` 在状态卡下方新增系统代理开关卡片（`el-switch`），走与设置页相同的统一编排层（`proxyApi.setSystemProxy` → 持久化意图 + 写注册表 + **托盘图标变色**），成功后同步 store 的 `system-proxy` 字段避免整包保存覆盖。i18n 新增 `dashboard.system_proxy_hint`。
12. **订阅「更新」按钮 + 人工优选去 DIRECT**：
   - **订阅更新**：`import_profile_from_url` 写入时在文件顶部持久化 `# subscribe-url: <url>` 注释头；新增命令 `update_profile_subscription`——读回 URL → 重新拉取（30s 超时、UA `ClashEdge/0.8.5`）→ 校验 YAML（**失败不覆盖**原文件）→ 写回（保留注释头）→ 若为激活中的 Profile 走统一编排层 `activate_profile` **热重载生效**。`list_profiles` 返回 `url` 字段，前端配置页仅对含订阅地址的 profile 显示「更新」按钮（i18n `profiles.update`）。`commands/profiles.rs` 新增 `extract_subscribe_url` 辅助函数。
   - **人工优选去 DIRECT**：`core/config.rs` 注入逻辑改为人工优选只注入真实节点、自动优选保留 DIRECT 兜底，详见第 7 项。
13. **恢复「配置」导航页 + 概览精简（最终布局）**：
   - **配置页恢复**：重建 `src/views/ProfilesView.vue`（页面级：工具栏「**订阅管理 / 新建配置 / 导入配置 / 导出配置**」四按钮直连 + 配置文件卡片列表 + 全部对话框），恢复 `/profiles` 路由、侧栏「配置」菜单项（App.vue）与 i18n `nav.profiles` 键；`profiles.title` 改回「配置文件管理」，新增 `profiles.subscribe_manage`、删除不再使用的 `profiles.manage` 与「更多▾」下拉。原 `src/components/SubscriptionManager.vue`（对话框版）已删除，逻辑并入 ProfilesView。
   - **概览精简**：`DashboardView.vue`——状态卡内**同卡均分三列**排布核心控制按钮「启动/停止 → **重启核心** → **重载配置**」（`.core-actions` `grid repeat(3,1fr)`），去掉快捷栏的「订阅/新建」入口与订阅管理入口行；设置卡只保留**系统代理开关**。移除 SubscriptionManager 与 profilesStore 引用。
14. **托盘猫咪配色（开=#15803D 绿 / 关=#2A62CC 蓝）**：`build_tray_icon` 按目标色着色，亮度因子 `f = 0.6 + 0.4·l/255` 映射到 [0.6, 1.0]——最高亮像素即目标色本身、暗部为目标的 60%，色相与目标色严格一致，浅色/深色任务栏均清晰可辨。详见第 6 项。
15. **人工优选手动测速按钮图标化**：`ProxiesView.vue` 中「人工优选」组标题行的手动测速从带文字按钮改为**图标圆形按钮**（`@element-plus/icons-vue` 的 `Lightning` 图标，`:title="proxies.manual_test"` 提示），与右侧组延迟的 `Refresh` 刷新小圆钮区分（`group-actions` flex 容器），保留 `testingNodes` loading 态。
16. **配置卡片操作顺序 + 概览核心控制细化**：`ProfilesView` 卡片按钮固定顺序「**更新 → 删除 → 重命名 → 原始编辑**」（激活按钮仅非激活时排最后）；`DashboardView` 核心控制改为**均分三列**且顺序「启动/停止 → 重启核心 → 重载配置」。
17. **UI 配色重构为设计系统 token（含未用 token 精简）**：`styles.css` 重写为设计系统——背景/文本/边框/强调/状态色 token 全套，浅色 `:root` 与深色 `html[data-theme="dark"]` 两套；Element Plus 六色经 `color-mix()` 按 EP 混合比例映射到 `--accent/--done/--approval/--error/--idle`。深色 accent 为浅蓝 `#77A7FF`，主按钮/单选/plain/text/link 深色态文字改深色 `#101214` 保证对比度（WCAG）。主题机制：`theme.ts`/`main.ts` 同时维护 `dark` class（EP 深色 css-vars 依赖 `html.dark`）与 `data-theme` 属性（设计系统依赖）；窗口原生底色 `setBackgroundColor` 与 `--bg-app` 同步（tauri.conf `backgroundColor` 同改）。全部组件（App.vue + 6 视图）改用 token，审计确认无硬编码色值残留；托盘配色同设计系统（见第 6/14 项）。**精简 16 个零引用 token**：`--text-inverse/--border-default/--divider/--accent-hover/--planning(-bg)/--running(-bg)/--approval-bg/--done-bg/--error-bg/--idle-bg/--focus/--shadow-sm/--shadow-md/--r-lg`（仅删当前 UI 与 EP 映射均未消费者；状态色保留 `--done/--approval/--error/--idle` 本体）。


18. **便携模式加固（0.8.5 后期，2026-08-19）**：修复「mihomo not found: %APPDATA%\...」回退陷阱，落实 App/Data/Other 三分离：
   - **`util/paths.rs`**：新增纯函数 `portable_indicators(exe_dir)`——`App/portable.dat` 存在**或** `App/clash-edge-core.exe` 存在（复制/改名/换盘符后 marker 丢失仍自愈判定）；`get_mihomo_path` 便携模式下 mihomo **固定** `<exe_dir>/App/clash-edge-core.exe`，缺失即报错、**不再回退数据目录/sidecar**（消灭 %APPDATA% 静默回退）；新增 `mihomo_missing_hint` 给出可操作提示（便携→检查 App/ 内核是否随包；安装版→提示 sidecar 打包）。
   - **`core/manager.rs`**：新增 `init_error` 字段——内核缺失不阻断应用启动（否则 UI 起不来、用户看不到原因）；`start()` 时以 `CoreStatus::Error(可操作提示)` 呈现。
   - **`util/logging.rs`**：便携模式日志写入 `<exe_dir>/Data/logs`（随包迁移），不再散落 OS 日志目录；目录不可用回退 OS 日志目录。
   - **`util/autostart.rs` + `main.rs`**：新增 `parse_launcher_path`/`paths_equal`/`repair_autostart`——便携包整体移动/改名后，启动时自动把注册表 Run 键自启路径重写为当前 exe 位置（仅便携模式）。
   - **`tools/build-portable.ps1`**：加前置校验（4 个 sidecar 源存在、release exe 非启动器残片 >5MB）与后置断言（根 ClashEdge.exe / App/portable.dat / App/clash-edge-core.exe 等 7 项齐备、portable.dat 为空文件），失败即中止打包。
   - 单测：`util::paths::tests`（5 例）+ `util::autostart::tests`（7 例）；`cargo test` **34/34 通过**（此前 22/22）。
   - 已删除 `tools/ClashEdge.exe`（~8.7KB 历史 C# 启动器残片，2026-08-19）——与真实应用同名、运行即复现上述 bug，已被本版本原生便携布局取代；`.cs` 源码留档参考。

### 实机验证结果（0.8.5 打包后）

- 前端 `npm run build` 通过；`cargo test` **22/22 通过**。
- 空闲端口启动实测：内核监听正常、无绑定冲突；控制器实时查询确认 6 组结构正确（GLOBAL + 5 组）、4 个规则集全部加载（direct=111484 / proxy=27053 / media=1578 / **ai=81**）。
- **本轮（恢复配置页 + 概览精简 + 托盘配色 + 测速图标化）构建验证**：`vue-tsc --noEmit` 零错误，Vite 生产构建产出 `ProfilesView` / `DashboardView` chunk；`cargo test` 22/22 通过。打包产物 `release/ClashEdge-portable-0.8.5-win64.zip`（35.9 MB）为叠加本轮全部改动后**重新打包**，SHA256 `F046C86E8F3B240D2B695F8D7E4EF83D3BAE536A585217093B7AA3EB37B04295`，覆盖上一版包。

## 5. 清理记录

### 2026-08-19（便携加固同日）

| 删除项 | 大小 | 原因 |
| --- | --- | --- |
| 根目录 `ClashEdge.rar` | 1.78 GB | 源码零引用、gitignore 未覆盖的历史归档（已确认用户不再需要） |
| 根目录 `vue-tsc-out.txt` | 0 B | 某次 `vue-tsc --noEmit` 空输出残留 |
| `src-tauri/icons/android/`、`ios/`（34 文件） | ~400 KB | Tauri 模板残留，移动平台图标，本项目仅 Windows 便携 |
| `icons/icon.png`、`64x64.png`、`128x128(.png@2x)` | ~10 KB | 模板默认图标，未被引用 |
| `icons/cat-512x512.png`、`cat-icon.png` | ~17 KB | cat 系列仅 `cat-32/128/256` 被 `tauri.conf.json` bundle.icon 引用 |
| `icons/Square*.png`（9 个）、`StoreLogo.png` | ~145 KB | Windows Store / 打包平台模板图标，未被引用 |

**保留**（构建关键文件，均被 `tauri.conf.json` / `build.rs` / `tray/builder.rs include_bytes!` 引用）：`cat-32x32.png`、`cat-128x128.png`、`cat-256x256.png`、`cat.icns`、`cat.ico`、`32x32.png`、`icon.ico`、`icon.icns`。

**用户确认保留**：`portable-template/.../static/imgs/logo_64*.png`（2 个）、`tools/ClashEdge.ico`。

**验证**：删除后 `cargo build --release` 关键图标文件齐全（`include_bytes!("../../icons/32x32.png")` 仍可编译）；此前 34/34 单测与 release 构建已通过，未改动任何 Rust/TS 源码。

### 2026-08-16（原记录）

按 GitHub 项目规范整理仓库，删除**核查确认零引用**的旧 Electron/R8.3 遗留（共释放 ~10.78 MB）。判定依据：全仓库搜索（排除 node_modules/target/dist）无任何代码、构建脚本、配置引用。

| 删除项 | 大小 | 原因 |
| --- | --- | --- |
| `portable-template/.../static/files/default/`（Country.mmdb） | 4.0 MB | 打包脚本不拷贝的重复地理数据 |
| `.../win/common/sysproxy.exe` | 102 KB | 系统代理已改 Rust winreg 直写注册表 |
| `.../win/common/schtasks.xml`、`service.yml` | 1.3 KB | 旧服务/计划任务配置，零引用 |
| `.../win/common/tun2socks/`（TAP 驱动套件） | 0.9 MB | 旧 tun2socks 方案，零引用 |
| `.../win/x64/service/`（clash-core-service.exe、service.exe） | 6.1 MB | 旧 Windows 服务，自启动已改根启动器 |
| `tools/asar-tool.cjs`、`pack-asar.cjs` | 3.7 KB | Electron ASAR 打包工具 |
| `tools/patch-r8-*.cjs`（3 个） | 12 KB | 旧 Electron 补丁，逻辑已由 Rust 重写 |
| `tools/make-launcher-icon.ps1` | 5 KB | 零引用 |
| `src/assets/typescript.svg`、`vite.svg` | 模板残留 | 未引用（`tauri.svg` 为 favicon 保留） |

文档整理：根目录 4 份 .md 归入 `docs/`；过期的 `R8.3-REVISION-NOTES.md` 移入 `docs/archive/`；新增根 `README.md`、根 `.gitignore`；`tauri-scaffold/README.md` 由模板内容改为指向根 README。

## 6. 已知问题与注意事项

- **旧版 Clash.F.Win 端口冲突（重点）**：若旧版 Clash.F.Win（Electron）仍在运行，会占用 `127.0.0.1:7890` 与 DNS `9053`，新内核绑定失败。新版本会如实报错（顶部红条），**不会**再假装运行中。用户需先关闭旧版。
- **规则集网络刷新**：`direct/proxy/media/ai/ad` 为 HTTP 型 rule-provider，首次用内置本地文件；若机器无法直连 `raw.githubusercontent.com`（无可用代理），刷新会失败但保留本地已加载规则，不影响启动。
- **构建仍打包、但运行时未调用的侧车**：`go-tun2socks.exe`（2.8 MB）、`EnableLoopback.exe`（75 KB）。TUN 实际走 mihomo 原生 `tun.enable`（wintun.dll），Rust 从不启动这两个 sidecar。**保留以防 TUN 回归，未删除**；后续确认无用时，可连同 `build-portable.ps1` 的拷贝行一起移除（精简候选）。
- **启动器说明（重要，勿再误记为"已废弃"）**：便携包**根目录的 `ClashEdge.exe` 就是 C# 启动器**——`tools/build-portable.ps1` 每次打包都会用 .NET Framework 的 `csc.exe` 把 `tools/ClashEdge.Launcher.R8.2.cs` 编译为 `portable-out/ClashEdge.exe`（带 cat.ico 图标），由它拉起 `App/ClashEdge/ClashEdge.exe`（Tauri 应用本体）。2026-08-19 删除的是仓库里残留的历史编译产物 `tools/ClashEdge.exe`（~8.7KB），`.cs` 源码与编译流程至今仍在使用。
- **开发卫生约定**：终止进程必须 PID + ExecutablePath 双重校验（且仅限测试目录）；任何文件不得残留订阅地址/密钥等非程序必需数据；不允许通过隐藏错误/吞异常/删功能让构建假成功。

## 7. Phase 0 基线（0.8.7 审计与发布门禁）

2026-08-22 基于 commit `9c6b203`（v0.8.7）完成发布级审查，产出两份基线文档，是后续所有开发的必读材料：

- **`docs/AUDIT-0.8.7.md`** —— 审查报告：总体结论表、P0 问题清单（配置迁移静默丢数据、先改内存后落盘、Settings 假保存、reload 吞错、崩溃自愈不恢复系统代理、多 watcher 竞态、崩溃循环计数清零）、P1 清单、工程目标六条硬原则、分阶段路线图（Phase 1 = 0.8.8 状态一致性，6 个 PR）。
- **`docs/RELEASE-GATE.md`** —— 发布前逐项实测清单：构建校验 / 生命周期与状态一致性 / 网络安全 / Windows 环境恢复 / 便携包 / 更新链路 / UI 与国际化，含 1.0 测试矩阵全量场景与当前已知阻断项（Settings 文件导入的 fs 权限问题）。

约定：任何新会话接手开发前，先读这两份文档；修复 P0 时以 AUDIT 的验收标准为准；发版前必须过 RELEASE-GATE。

## 8. 当前架构要点

- **AppConfig 与运行时配置分离**：`Data/config.yaml` 是应用配置（AppConfig，含 profile 激活名 / locale / geodata-mode 等），mihomo 实际加载的是由 `core::config::build_runtime_config`（AppConfig + 激活 Profile 合并）生成并原子写入的 `Data/runtime-config.yaml`。应用配置不再直接喂给内核。
- **单一数据源共享 Arc**：`ConfigManager` 持有 `Arc<parking_lot::RwLock<Config>>`，`CoreManager` 共享同一个 Arc——所有读取/修改走同一把锁，托盘、命令层、watcher 读到的都是同一份状态。已知缺陷见 AUDIT P0-2（set_config 先改内存后落盘）。
- **runtime.rs 统一编排层**：`core/runtime.rs` 提供 `apply_proxy_mode` / `apply_tun` / `apply_system_proxy` / `refresh_tray` 等入口，前端命令与托盘事件都走这里，统一遵循「校验 → 持久化 → 重生成 runtime-config → 实时下发 → 失败回滚 → 推送事件刷新托盘」链路。
- **便携布局**：便携包根目录为 `ClashEdge.exe`（C# 启动器，csc 编译）+ `App/`（Tauri 应用本体 clash-edge-core.exe sidecar + DefaultData + portable.dat 标记）+ `Data/`（用户数据：config.yaml / runtime-config.yaml / profiles / logs / 规则集）+ `Other/`（附属资源）。portable.dat 或 App 内核存在即判定便携模式，mihomo 固定解析 `<exe_dir>/App/clash-edge-core.exe`，无 %APPDATA% 回退。

## 9. 协作文档

- 多智能体协作规则见 `.claude/CLAUDE.md`（默认并行优先，探索/测试/Review 优先并行，核心修改由单一 Agent 实施）。
- 本文件为当前事实来源；`docs/archive/` 内文档不代表当前实现，仅追溯参考。
