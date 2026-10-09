# ClashEdge UI 重构 · AI 开发指令（Clear Edge v2）

> 给 AI 编码代理（Claude Code / Codex / Cursor 等）执行的开发说明。
> 一次只执行**一张任务卡**，每张卡对应一个独立 PR。
> 设计画布（人类评审用，代理不需要访问）：https://claude.ai/artifact/DfCrSvmWdyqQGC4q8TnHKS
> 本文件是自包含的：实现所需的数值都写在这里。

---

## 0. 开工前必读

1. 先完整读完本文件的 §1–§4，再读你领到的任务卡（§6）。
2. 优先级：**本文件 > 设计画布 > docs/DESIGN_SYSTEM.md v1.0**。v1.0 已被本设计取代，两者冲突时以本文件为准。v1.0 中「圆角 ≤ 12」「只用 ease 曲线」「禁止缩放动画」等规则不再适用。
3. 动手前用 Grep/Read 确认任务卡里提到的文件、函数、store 接口确实存在；不存在时停下来报告，不要猜。
4. 每张卡完成后运行 §7 的验收命令，**如实报告结果**；失败时附上输出，不要写成「应该可以」。

---

## 1. 项目事实（以代码为准）

| 项 | 事实 |
|---|---|
| Windows 客户端 | `apps/windows`：Tauri 2 + Vue 3.5 + Pinia + vue-i18n 11 + **Element Plus 2.x（按需注册，见 `src/element-plus.ts`）**。已正式发布 |
| Android 客户端 | `apps/android`：Kotlin + Jetpack Compose（Material 3）。**实验性**：没有 gradle wrapper，内核是占位实现，无法在本地构建验证 |
| macOS / Linux / iOS | 仓库中不存在，不在本次范围内 |
| 主题入口 | `src/theme.ts`：同时维护 `<html data-theme>`（设计系统用）和 `.dark` class（EP 用）；存储键 `cfw-theme`；目前默认值为 `"dark"` |
| 样式入口 | `src/styles.css`：token 段 + EP 映射 + 全局精修，约 510 行 |
| 路由 | `src/router/index.ts`：hash 模式；路由 `/dashboard` `/proxies` `/profiles` `/connections` `/logs` `/settings` |
| 导航 | `src/App.vue` 中的 `navItems`，图标来自 `@element-plus/icons-vue`；`matchMedia("(max-width: 749px)")` 时侧栏收起为 64px |
| 文案 | 唯一来源是 `src-tauri/resources/i18n/zh-CN.yaml` 和 `en-US.yaml`（托盘与前端共用），前端通过 `get_i18n_messages` 获取。**新增文案必须两种语言都加** |
| 流量数据 | `get_connections` 返回 `download_total` / `upload_total`（累计字节）。实时速率 = 相邻两次轮询的差值 ÷ 间隔，**不需要新增后端接口** |
| 规则数据 | 目前**没有**读取规则的命令。规则页需要新增 Rust 命令，通过 `src-tauri/src/core/controller.rs` 访问 mihomo 外部控制器的 `GET /rules` |
| CSP | `font-src 'self' data:`，`connect-src 'self'`。**禁止使用 Google Fonts 或任何 CDN**，字体必须随包附带 |
| 质量门 | `scripts/ci/quality.ps1`：cargo fmt / clippy -D warnings / cargo test / npm test / npm run build / npm audit 等 |
| 代理组（不可改动） | **扶梯出行 · 人工智能 · 影音视听 · 人工优选 · 自动优选**（另有隐藏的 GLOBAL）。名称、数量、类型、顺序以及 Rust 侧生成逻辑一律不动 |

---

## 2. 硬性约束

**必须（MUST）**
- 视图和组件的 `<style scoped>` 里只能引用 `--ce-*` Token 和布局属性。**禁止出现 HEX、rgb()、字号字面量、圆角字面量**（`0`、`50%`、`999px` 除外）。
- 每个新文案都同时加进 `zh-CN.yaml` 和 `en-US.yaml`。
- 所有可交互元素都使用原生 `<button>`、`<a>`、`<input>` 或 EP 组件。纯图标按钮必须有 `aria-label`。键盘焦点环统一为 `2px solid var(--ce-accent-bg)`，偏移 2px。
- 所有数值（速率、延迟、流量、端口、计数）使用 `font-variant-numeric: tabular-nums`。端口、IP、URL、日志使用 `--ce-font-mono`。
- 危险操作走现有的 `useConfirm`。确认按钮写明具体动作（例如「删除订阅」），默认焦点放在「取消」。
- 状态不能只靠颜色区分：至少同时用颜色 + 形状或文字。
- 保持现有测试通过；行为发生变化时同步更新对应的 `*.spec.ts`，不要删除测试来让 CI 变绿。

**禁止（MUST NOT）**
- 改动代理组（见 §1 最后一行）。
- 删除任何现有功能，包括：启动/停止/重启核心、重载配置、系统代理、TUN、订阅的更新/重命名/原始编辑/User-Agent/导入/导出/删除、设置中的每一项。只能移动位置，不能删除。
- 引入新的 UI 框架或 CSS 框架（例如 Tailwind、UnoCSS）。图表用手写 SVG，不引入图表库。
- 使用毛玻璃 / `backdrop-filter`、渐变、装饰性大阴影或 emoji 图标。
- 在一张任务卡里顺带重构与本卡无关的代码。

---

## 3. Design Tokens（权威数值）

### 3.1 颜色 · 浅色 / 深色

| Token | 浅色 | 深色 | 用途 |
|---|---|---|---|
| `--ce-bg-page` | `#F5F6F8` | `#0F1115` | 页面底 |
| `--ce-bg-surface` | `#FFFFFF` | `#171A1F` | 卡片、列表组 |
| `--ce-bg-sidebar` | `#FFFFFF` | `#171A1F` | 侧栏（右边 1px `--ce-border`） |
| `--ce-bg-elevated` | `#FFFFFF` | `#1F2329` | 弹层、菜单、对话框 |
| `--ce-fill-hover` | `#F5F6F8` | `#1F2329` | 悬停底 |
| `--ce-fill-soft` | `#EDEFF2` | `#2A2F36` | 按下底、分段控件轨道、搜索框底 |
| `--ce-border` | `#E2E5EA` | `#2A2F36` | 描边 |
| `--ce-divider` | `#EDEFF2` | `#2A2F36` | 分割线 |
| `--ce-text-primary` | `#15171B` | `#ECEEF1` | 主文字 |
| `--ce-text-secondary` | `#4A515C` | `#A9B0BB` | 次文字 |
| `--ce-text-tertiary` | `#666E7B` | `#8B93A0` | 三级文字、表头、占位 |
| `--ce-text-disabled` | `#A3AAB6` | `#5A616C` | 禁用 |
| `--ce-fill-disabled` | `#E2E5EA` | `#2A2F36` | 禁用填充 |
| `--ce-accent-bg` | `#2465F0` | `#2465F0` | **只用作填充**：主按钮、开关开启、选中单选，配白字（5.01:1） |
| `--ce-accent-bg-hover` | `#1C55D4` | `#1C55D4` | |
| `--ce-accent-bg-press` | `#1746B0` | `#1746B0` | |
| `--ce-accent-fg` | `#1C55D4` | `#5B92FF` | **只用作文字或图标**：链接、选中项文字、文字按钮 |
| `--ce-accent-soft` | `#EBF1FE` | `#16243F` | 选中浅底 |
| `--ce-on-accent` | `#FFFFFF` | `#FFFFFF` | |
| `--ce-success-solid / -fg / -soft` | `#1E9E5A` / `#157A46` / `#E8F6EE` | `#3FCB7E` / `#3FCB7E` / `#122A1E` | 已连接、延迟良好 |
| `--ce-warning-solid / -fg / -soft` | `#F0A01E` / `#9A5700` / `#FEF4E2` | `#F5B544` / `#F5B544` / `#2E2412` | 延迟一般、即将过期 |
| `--ce-danger-solid / -fg / -soft` | `#E5484D` / `#CC2F35` / `#FDECEC` | `#FF6B6F` / `#FF6B6F` / `#331A1C` | 失败、延迟较差、危险操作 |
| `--ce-danger-bg` | `#CC2F35` | `#CC2F35` | 危险按钮填充（白字 5.22:1） |
| `--ce-pending-solid / -fg / -soft` | `#7B5CF5` / `#6542E6` / `#F1EDFE` | `#A08CFF` / `#A08CFF` / `#231D3D` | 启动中、测速中、更新中 |
| `--ce-idle` | `#4A515C` | `#A9B0BB` | 未运行 |
| `--ce-chart-down` | `#2465F0` | `#4A80F5` | 下载曲线 |
| `--ce-chart-up` | `#0891B2` | `#19A6B5` | 上传曲线 |
| `--ce-chart-grid` | `#EDEFF2` | `#2A2F36` | 网格线 |
| `--ce-switch-off` | `#7D8592` | `#3A4049` | 开关关闭轨道 |
| `--ce-toast-bg` / 文字 | `#2B2F36` / `#FFFFFF` | `#CDD2D9` / `#15171B` | Toast（ElMessage） |
| `--ce-scrim` | `rgb(21 23 27 / .32)` | `rgb(0 0 0 / .56)` | 遮罩 |

规则：`*-solid` 只能用于色点、图标、图表线条，**不能用于文字**（黄色和绿色的实色达不到 4.5:1）。彩色文字一律用 `*-fg`。

### 3.2 字体与字号

```css
--ce-font-sans: "Segoe UI Variable Text", "Segoe UI", "Microsoft YaHei UI", "Microsoft YaHei",
                -apple-system, "PingFang SC", "Noto Sans CJK SC", sans-serif;
--ce-font-mono: "JetBrains Mono", "Cascadia Code", Consolas, monospace;
/* JetBrains Mono（OFL 协议）放在 src/assets/fonts/，用 @font-face 引入，并在 THIRD_PARTY_NOTICES.md 中登记 */
```

| Token | 桌面 字号/行高/字重 | 用途 |
|---|---|---|
| `--ce-type-display` | 40 / 48 / 700 | 首页速率等关键数值（只用于数值） |
| `--ce-type-title-1` | 28 / 36 / 600 | 页面标题 `.page-title` |
| `--ce-type-title-2` | 20 / 28 / 600 | 区块标题、对话框标题 |
| `--ce-type-title-3` | 16 / 24 / 600 | 卡片标题 |
| `--ce-type-body` | 14 / 22 / 400（按钮 500） | 正文 |
| `--ce-type-callout` | 13 / 20 / 400 | 辅助说明 |
| `--ce-type-caption` | 12 / 16 / 400·500 | 标签、表头、时间 |
| `--ce-type-mono` | 13 / 20 / 400 | 日志、规则、IP |

### 3.3 形状、间距、阴影、动效、尺寸

```css
--ce-radius-xs: 6px;  --ce-radius-sm: 8px;  --ce-radius-md: 12px;
--ce-radius-lg: 16px; --ce-radius-xl: 20px; --ce-radius-full: 999px;
/* 按钮、输入框 = md · 卡片、节点格 = lg（节点格 14 视为 lg 的内缩）· 对话框 = xl · 延迟标签 = sm */

--ce-space-1: 4px; --ce-space-2: 8px; --ce-space-3: 12px; --ce-space-4: 16px; --ce-space-5: 20px;
--ce-space-6: 24px; --ce-space-8: 32px; --ce-space-10: 40px; --ce-space-12: 48px; --ce-space-16: 64px;

--ce-shadow-e1: 0 1px 2px rgb(21 23 27 / .06), 0 0 0 1px rgb(21 23 27 / .06);
--ce-shadow-e2: 0 8px 24px rgb(21 23 27 / .08), 0 0 0 1px rgb(21 23 27 / .06);   /* 菜单、Toast */
--ce-shadow-e3: 0 24px 64px rgb(21 23 27 / .16), 0 0 0 1px rgb(21 23 27 / .06);  /* 对话框 */
/* 深色：e2 = 0 8px 24px rgb(0 0 0 / .5), 0 0 0 1px #2A2F36；e3 = 0 24px 64px rgb(0 0 0 / .6), 0 0 0 1px #2A2F36 */
/* 卡片默认不加阴影：1px --ce-border 描边 + surface 底 */

--ce-ease-standard: cubic-bezier(.2, 0, 0, 1);
--ce-ease-enter: cubic-bezier(.05, .7, .1, 1);
--ce-ease-exit: cubic-bezier(.3, 0, .8, .15);
--ce-ease-spring: cubic-bezier(.34, 1.36, .64, 1);   /* 仅用于开关拇指和主按钮 */
--ce-dur-instant: 80ms; --ce-dur-fast: 140ms; --ce-dur-base: 220ms; --ce-dur-slow: 320ms;
/* 保留现有 prefers-reduced-motion 全局降级 */

--ce-control-h: 36px; --ce-control-h-sm: 28px; --ce-control-h-lg: 44px;
--ce-row-h: 44px; --ce-icon: 20px; --ce-icon-stroke: 1.75;
--ce-sidebar-w: 232px; --ce-sidebar-w-collapsed: 64px;
```

### 3.4 延迟分级（全局统一，设置中可调阈值）

| 档位 | 条件 | 色点形状 | 颜色组 | 显示 |
|---|---|---|---|---|
| 良好 | ≤ 200 ms | 圆 | success | `86 ms` |
| 一般 | 201–500 ms | 菱形（旋转 45° 的方块） | warning | `320 ms` |
| 较差 | > 500 ms | 横杠 | danger | `860 ms` |
| 超时 | 超时或失败 | 无 | `--ce-fill-soft` 底 + secondary 文字 | `超时` |
| 测速中 | — | 转圈 | pending | `测速中` |
| 未测 | 无数据 | 无 | 1px 虚线描边 + tertiary 文字 | `– ms` |

读屏文案：「延迟 86 毫秒，良好」。

---

## 4. 组件规范（摘要）

| 组件 | 规格 | 状态 |
|---|---|---|
| 按钮 | 高 36，水平内边距 16，圆角 md，字号 14/500；类型 primary / default / text / danger | 悬停 = `*-hover`；按下 = `*-press` + `scale(.98)`；禁用 = `--ce-fill-disabled` 底 + `--ce-text-disabled` 字；**一个视图最多一个 primary** |
| 开关 | 44×26，拇指 22，关闭轨道 `--ce-switch-off`，开启轨道 `--ce-accent-bg` | 需要提权的开关（TUN）先显示 pending 色「切换中」，授权完成后才落定，失败时弹回原位并给 Toast |
| 分段控件（新） | 轨道 `--ce-fill-soft`、内边距 3、圆角 full；选中段为 surface 底 + e1 | 用 `role="radiogroup"`，方向键可切换；用于代理模式、主题、规则视图 |
| 卡片 | surface 底 + 1px border + 圆角 lg + 内边距 20–24 | **卡片里不再嵌套卡片**，内部分组用 `--ce-divider` |
| 节点格 NodeTile（新） | 最小宽 200，内边距 12/14，圆角 14；第一行名称，第二行协议 + 延迟标签 | 悬停 = 描边 N300 + e1；选中 = 1.5px `--ce-accent-bg` 描边 + 右上角勾 + 名称用 `--ce-accent-fg`；不可用 = 文字变为 disabled |
| 列表行 | 高 44，左右内边距 16，图标 20 | 悬停 = fill-hover；按下 = fill-soft；选中 = accent-soft 底 + accent-fg 字 + 勾；危险项放在分组最后，用 danger-fg |
| 延迟标签 LatencyTag（新） | 高 22，圆角 sm，等宽 12/500，按 §3.4 分级 | 可点击：单击重新测该节点 |
| 输入框 | 高 36，圆角 md，标签放在上方 | 焦点 = accent 描边 + `0 0 0 3px rgb(36 101 240 / .18)`；错误 = danger 描边 + 下方 12px danger-fg 提示（失焦后才显示） |
| 对话框 | 宽 400，内边距 24，圆角 xl，e3 | 危险对话框：顶部 40px danger-soft 圆形图标；按钮顺序 [取消][删除…]（Windows 习惯） |
| Toast（ElMessage） | 高 44，圆角 md，`--ce-toast-bg`，e2，位置底部居中 | 普通 3s，带操作 5s，错误 6s，悬停时暂停计时 |
| 侧栏导航项 | 高 40，圆角 10，图标 20 | 选中 = accent-soft 底 + accent-fg 字 + 500 字重（不加左侧色条） |
| 流量图 TrafficChart（新） | SVG，线宽 2，只有下载线加 7% 面积填充，60 秒窗口，每秒更新，不做补间 | 悬停显示十字线 + Tooltip（数值用中性色，色点标识系列）；提供「查看数据表」；只有一个 Y 轴 |
| 主连接按钮（新） | 直径 96 | 停止 = surface + border；启动中 = pending-soft + 6px pending 外环；运行中 = accent-bg + 8px accent-soft 外环；异常 = danger-soft + danger 描边 |

图标：`lucide-vue-next`，尺寸 20，`stroke-width="1.75"`，只用线性图标。导航图标映射：首页 `House`、代理 `Globe`、订阅 `Layers`、规则 `ListFilter`、连接 `ArrowDownUp`、日志 `FileText`、设置 `SlidersHorizontal`。

---

## 5. 信息架构（新）

侧栏顺序：**首页 / 代理 / 订阅 / 规则 / 连接 / 日志 / 设置**

| 导航 | 路由 | 视图文件 | 变化 |
|---|---|---|---|
| 首页 | `/dashboard`（保留路由） | `DashboardView.vue` | 由「概览」更名；新增状态核心、快捷切换、流量卡 |
| 代理 | `/proxies` | `ProxiesView.vue` | 折叠列表改为「策略组列表 + 节点格」主从双栏 |
| 订阅 | `/profiles`（保留路由） | `ProfilesView.vue` | 由「配置」更名；功能不变 |
| 规则 | `/rules`（新增） | `RulesView.vue`（新增） | 只读 |
| 连接 | `/connections` | `ConnectionsView.vue` | 顶部加流量图 |
| 日志 | `/logs` | `LogsView.vue` | 级别筛选芯片 + 搜索 |
| 设置 | `/settings` | `SettingsView.vue` | 5 个 Tab 改为单页分组 + 搜索 |

更名只修改 i18n 的值（例如 `nav.dashboard: "首页"`），**不改路由路径和 i18n 键名**，避免影响托盘和测试。

---

## 6. 任务卡

> 每张卡格式：目标 / 涉及文件 / 步骤 / 验收标准。卡与卡之间有依赖时会注明。

### T1 · Token 基础设施（无依赖）

- **目标**：让 `--ce-*` Token 生效，并让旧变量名通过别名自动映射，切换一次就能看到新配色，视图零改动。
- **文件**：新增 `design/tokens.json`、`scripts/tokens/build.mjs`、`apps/windows/src/styles/tokens.css`（生成）、`apps/windows/src/styles/legacy-aliases.css`；修改 `src/styles.css`、`src/main.ts`（引入顺序）。
- **步骤**
  1. 把 §3 的全部数值写进 `design/tokens.json`，按 `{ "light": {...}, "dark": {...}, "global": {...} }` 组织。
  2. 写一个**零依赖**的 Node 脚本 `build.mjs`，读取 json，输出两样东西：
     - `tokens.css`：`:root`（global + light）和 `html[data-theme="dark"]`（dark），文件头写上 `AUTO-GENERATED — do not edit`；
     - Android 用的 `Tokens.kt`（供 T9 使用）。
     在 `apps/windows/package.json` 中加脚本 `"tokens": "node ../../scripts/tokens/build.mjs"`。
  3. `legacy-aliases.css`：把旧变量指向新 Token。`--bg-app→--ce-bg-page`，`--bg-surface→--ce-bg-surface`，`--bg-sidebar→--ce-bg-sidebar`，`--bg-raised→--ce-bg-elevated`，`--bg-soft→--ce-fill-soft`，`--interactive-hover→--ce-fill-hover`，`--border-subtle/--card-border→--ce-border`，`--text-*→--ce-text-*`，`--accent→--ce-accent-fg`，`--accent-soft/--menu-active-bg→--ce-accent-soft`，`--done→--ce-success-fg`，`--approval→--ce-warning-fg`，`--error→--ce-danger-fg`，`--idle→--ce-idle`，`--r-sm→--ce-radius-md`，`--r-md→--ce-radius-lg`，`--font→--ce-font-sans`。
  4. 从 `styles.css` 中删除原来的浅色/深色 token 段，改为 `@import` 上面两个文件（顺序：EP dark css-vars → tokens.css → legacy-aliases.css → 其余规则）。
  5. 用 `@font-face` 引入本地的 JetBrains Mono（woff2，Regular 和 Medium）。
- **验收**
  - `npm run build` 与 `npm test` 通过；
  - 两套主题下都没有出现未定义的变量（在 DevTools 中检查 `getComputedStyle(document.documentElement)`，所有 `--ce-*` 都有值）；
  - 用 `git grep -nE "#[0-9a-fA-F]{3,6}" apps/windows/src/styles/tokens.css` 核对，所有 HEX 都与 §3.1 一致。

### T2 · Element Plus 桥接与主题默认值（依赖 T1）

- **文件**：`src/styles.css`（EP 映射段）、`src/theme.ts`、`src/main.ts`。
- **步骤**
  1. 两套主题统一：`--el-color-primary: var(--ce-accent-fg)`。成功/警告/危险/信息分别映射到对应的 `*-fg`，light-3…9 / dark-2 继续使用现有的 `color-mix` 推导，但基色换成新 Token。
  2. 所有「填充」位置单独指回品牌色，并配白字：
     - `.el-button--primary`：`--el-button-bg-color`、`border-color`、`hover-*`、`active-*` 分别对应 `--ce-accent-bg`、`-hover`、`-press`；`text-color` 用 `--ce-on-accent`；
     - `.el-button--danger`（不含 plain/text/link）：使用 `--ce-danger-bg` + 白字；
     - `.el-switch`：`--el-switch-on-color: var(--ce-accent-bg)`，`--el-switch-off-color: var(--ce-switch-off)`；
     - `.el-radio-button` 选中态、`.el-checkbox` 选中态也指向 accent-bg，配白字；
     - `.el-message` 背景用 `--ce-toast-bg`。
  3. **删除** 3 段深色近黑文字覆盖：`html[data-theme="dark"] .el-button--primary`、`… .is-plain/.is-text/.is-link`、`… .el-radio-button`。
  4. `--el-border-radius-base: var(--ce-radius-md)`，`--el-border-radius-small: var(--ce-radius-sm)`，`.el-card`、`.el-dialog` 的圆角分别为 lg、xl。
  5. `.page-title` 改为 28/36/600，下边距 20。
  6. `theme.ts`：`getTheme()` 的兜底值从 `"dark"` 改为 `"system"`，**只影响没有保存过偏好的新用户**。说明：`tauri.conf` / `main.rs` 中窗口底色固定为深色，浅色系统下启动时可能出现一瞬间的深色闪屏——把窗口初始底色改为读取系统主题；如果改动超出本卡范围，就在 PR 里写明，作为后续事项。
- **验收**：两套主题下逐页截图，主按钮、开关、单选按钮、复选框都是品牌蓝底 + 白字；深色主题下的链接和选中文字是 `#5B92FF`；`npm test` 通过。

### T3 · 基础组件（依赖 T2）

- **新增** `src/components/ui/`：`CeSegmented.vue`、`CeLatencyTag.vue`、`CeNodeTile.vue`、`CeStatusCore.vue`（主连接按钮 + 状态文字）、`CeTrafficChart.vue`（SVG）、`CeSparkline.vue`。
- **修改** `StatusPill.vue`：改用色点 + 形状 + 文字的三重编码。
- 安装 `lucide-vue-next`（固定一个已发布的稳定版本号，并更新 lock 文件），然后替换 `App.vue` 中 `navItems` 的图标；如果 `@element-plus/icons-vue` 不再被任何文件引用，就移除这个依赖。
- 每个新组件都要有 `*.spec.ts`，至少覆盖：渲染、各状态对应的 class 和 aria、键盘交互（分段控件的方向键）。
- **验收**：§2 的规则全部满足；`npm test` 中新增测试通过；组件内没有字面量颜色（用 `git grep -nE "#[0-9a-fA-F]{3,6}|rgb\(" apps/windows/src/components/ui` 检查，结果应为空）。

### T4 · 首页 DashboardView（依赖 T3）

布局为两行 2:1 栅格（窗口宽度小于 960 时变为单列）：

1. **状态核心卡**（占 2/3）
   - 左侧：`CeStatusCore` 主按钮，对应现有的启动/停止核心动作。旁边显示状态文字：「已连接 / 核心已停止 / 启动中… / 异常」（核心运行且系统代理或 TUN 至少开启一个时，才算「已连接」），下一行是「mihomo {版本} · 混合端口 {port}」，再下一行是两个文字按钮：**重启核心**、**重载配置**（复用现有 action）。
   - 右侧：代理模式分段控件（规则 / 全局 / 直连，复用 `configStore` 的模式切换）；下方两行开关：**系统代理**（复用现有逻辑）和 **TUN 模式**（复用设置 › TUN 的同一 action，提权流程不变）。
   - 删除原来卡片里的三块统计子卡片，信息并入上面的文字。
2. **实时流量卡**（占 1/3）：下载和上传速率（display 字号，tabular-nums）+ `CeSparkline` + 底部一行「本次会话 ↓ x · ↑ y · n 个活动连接」。速率按 §1 的方式由 `get_connections` 的累计值求差得出。轮询沿用 `usePolling` 和 `connections` store 现有的频率策略，**不要新建第二个轮询**。
3. **快捷切换卡**（占 2/3）：标题行是「快捷切换」+ 策略组下拉（默认显示「扶梯出行」当前所指向的手动组，可在 5 个组之间切换；自动优选是测速组，切换到它时节点格只读），右侧有「测速」「全部节点 →」。下方是 `CeNodeTile` 网格，点击即调用现有的选节点 API。
4. **订阅卡 + 出口卡**（占 1/3）：显示当前订阅名称、用量进度条和到期时间（数据已有就显示，没有就隐藏这一行）。出口 IP 卡**只在已有对应数据源时才做**；没有数据源就跳过，并在 PR 中说明。

- **验收**：原有的启动、停止、重启、重载、系统代理功能在新界面上都能使用；`DashboardView.spec.ts` 已更新并通过；从首页切换节点 ≤ 2 次点击，切换模式 1 次点击。

### T5 · 代理页 ProxiesView（依赖 T3）

- 顶部工具栏：页面标题「代理」+ 搜索框（Ctrl+K 聚焦）+「全部测速」（primary）+ 刷新图标按钮。代理模式改用分段控件，放在标题下方。
- 主体分两栏：
  - 左栏是策略组列表（宽 240–300），每行显示组名、类型（手动 / 测速）、当前节点；
  - 右栏是选中组的节点格（`repeat(auto-fill, minmax(200px, 1fr))`）+ 地区筛选芯片 + 「按延迟排序」。
- 自动优选（URLTest）组的节点格只读，并显示「由测速自动选择」。
- 窗口宽度小于 960 时，左栏改为顶部横向芯片。
- 保留现有的模式过滤逻辑：rule 模式显示 5 组并隐藏 GLOBAL；global 模式只显示 GLOBAL；direct 模式显示空状态说明。
- **验收**：组名、组数与现状完全一致；`ProxiesView.spec.ts` 已更新并通过。

### T6 · 订阅页 ProfilesView（依赖 T3）

- 更名只改 i18n 的值：`nav.profiles` 和页面标题改为「订阅」/「Subscriptions」。
- 工具栏：「新建配置」「导入 / 导出 ▾」（default 按钮）+「添加订阅」（**唯一的 primary**）。
- 卡片网格 `minmax(300px, 1fr)`：名称 + 状态标签（使用中 / 即将到期 / 更新失败，用 soft 底 + fg 字）、URL（等宽字体，单行截断）、用量进度条、「N 个节点 · x 前更新」，以及一个主操作（更新 / 使用 / 原始编辑）。
- 「⋯」菜单收纳：重命名、原始编辑、User-Agent、导出、删除…（删除走 useConfirm）。
- 去掉现有卡片上「更新」按钮的绿色描边和「删除」按钮的红色描边。
- **验收**：`ProfileCard.spec.ts` 和 `profiles` 相关测试通过；所有现有对话框（Subscribe / Rename / RawEdit / UserAgent / Export / Content）都能从新界面打开。

### T7 · 规则页 RulesView（新增，依赖 T3）

- **Rust**（完全照搬 `get_connections` 的链路，不另起炉灶）：
  1. 在 `src-tauri/src/core/controller.rs` 的 `ControllerClient` 上新增 `pub(crate) async fn get_rules(&self)`，写法参照同文件的 `get_proxy_groups`（用 `api_url(&["rules"], None)`、`api_headers()`、`api_client()`），返回 `Vec<serde_json::Value>`，只保留 `type`、`payload`、`proxy` 三个字段；
  2. 在 core manager 上暴露同名方法，方式与 `get_connections` 相同；
  3. 新增 `src-tauri/src/commands/rules.rs`，结构照抄 `commands/connections.rs`（核心未初始化时返回 `InvalidState`），在 `commands/mod.rs` 中声明，在 `main.rs` 的 `tauri::generate_handler![…]` 中注册；
  4. 补充单元测试，覆盖字段裁剪和空列表；必须通过 clippy `-D warnings`。
- **前端**：新增 `src/api/rules.ts`、`src/views/RulesView.vue`，并添加路由 `/rules`，在 navItems 中插在订阅之后。
  - 分段控件切换「规则 {n} / 规则集」（规则集视图如果暂时没有数据来源，就先只做规则视图，分段控件隐藏）；
  - 搜索框按类型、内容、策略过滤；
  - 列表用网格行（不要用 `<table>` 套循环），行高 44，列为：序号 / 类型标签（等宽）/ 匹配内容（等宽，截断）/ 策略（DIRECT 用 secondary，REJECT 用 danger-fg，其他用 accent-fg）/ 命中数（如果 `/rules` 不提供命中数就省略这一列）；
  - 超过 500 条时使用虚拟滚动或分页，不要一次渲染全部。
  - 首期**只读**，不提供编辑入口。
- 新增 i18n 键 `nav.rules`、`rules.*`（中英文）。
- **验收**：cargo test、clippy、npm test、build 全部通过；核心未运行时显示空状态「核心未运行」。

### T8 · 连接 / 日志 / 设置（依赖 T3，可拆成 3 个 PR）

- **ConnectionsView**：顶部卡片放 4 个数值（下载、上传、活动连接、本次会话流量）+ `CeTrafficChart`（60 秒）。下方用分段控件切换「活动 / 已关闭」（如果 store 没有已关闭的数据，就只做「活动」）+ 筛选框。表格列：主机·进程、规则 → 链路、类型、↓、↑、时长、断开（图标按钮）。「全部断开…」使用 danger 文字按钮 + useConfirm。保留现有的截断提示。
- **LogsView**：级别筛选芯片（带计数；色点形状 DEBUG 无形状 / INFO 圆 / WARN 菱形 / ERROR 横杠）+ 正则搜索 + 暂停 / 导出 / 清空…；日志正文用 `--ce-font-mono` 13/20，三列（时间 | 级别 | 消息）；向上滚动时暂停自动跟随，出现「回到最新 ↓」。
- **SettingsView**：5 个 Tab 改为单页分组，分组名仍为 常规 / 代理 / TUN / 高级 / 关于，宽屏下分两列。复用 `components/settings/*Tab.vue` 的内容，但把「标签右对齐表单」改为列表行（左边标签，右边控件）。顶部加「搜索设置」：命中的项高亮，并滚动到对应位置。所有现有设置项一个都不能少。主题改用分段控件（浅色 / 深色 / 跟随系统）。
- **验收**：`SettingsView.spec.ts`、`SettingsA11y.spec.ts`、`ConnectionsView.spec.ts` 已更新并通过；所有设置项都能找到并修改。

### T9 · Android 主题对齐（可独立执行，但需要 T1 的生成脚本）

- 用 T1 生成的 `Tokens.kt` 替换 `ui/theme/Theme.kt` 中的配色：
  - Light：`primary=#1C55D4`、`primaryContainer=#EBF1FE`、`background=#F5F6F8`、`outline=#E2E5EA`；
  - Dark：`primary=#5B92FF`、`onPrimary=#0F1115`、`background=#0F1115`、`surface=#171A1F`、`surfaceVariant=#1F2329`；
  - 不启用 dynamicColor；
  - Shapes 设为 small 12 / medium 16 / extraLarge 20。
- 填充型主按钮统一用 `containerColor=#2465F0` + 白字（封装成 `CePrimaryButton`）。
- 新增 `LocalCeStatus`（CompositionLocal），替换 `HomeScreen.kt` 中写死的 4 个状态色。
- 底部导航收敛为 4 项：首页 / 代理 / 连接 / 更多；订阅、规则、日志、设置放进「更多」。如果对应页面还不存在，就只调整现有 Screen 的入口。
- **验收**：由于仓库缺少 gradle wrapper，**无法在本地构建**。PR 中必须写明「未编译验证」，并且只做语法上自洽的修改。不要因为本卡新增 gradle wrapper，那属于另一项工作。

### T10 · 文档与守护（最后执行）

- 把 `docs/DESIGN_SYSTEM.md` 升级为 v2.0（内容取本文件 §3–§5），并在开头注明 v1.0 已被取代。
- 在 `scripts/ci/quality.ps1` 中增加两项检查：
  1. 运行 `npm run tokens` 后，`git diff --exit-code` 必须没有差异（保证生成产物与源文件一致）；
  2. 在 `apps/windows/src/views` 和 `apps/windows/src/components` 下 grep HEX 或 rgb 字面量，出现即失败（`tokens.css` 和 `legacy-aliases.css` 除外）。
- 所有视图都迁移到 `--ce-*` 之后，删除 `legacy-aliases.css`。

---

## 7. 验收命令（Windows 端，在 `apps/windows` 下执行）

```bash
npm ci
npm run tokens
npm test
npm run build
```

```bash
cargo fmt --check --manifest-path src-tauri/Cargo.toml
```

```bash
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
```

```bash
cargo test --manifest-path src-tauri/Cargo.toml --all-targets
```

完整质量门：在仓库根目录用 PowerShell 运行 `.\scripts\ci\quality.ps1`。

### 每个 PR 的自检清单

- [ ] 没有改动代理组（名称、数量、类型、顺序）
- [ ] 没有删除任何现有功能，被移动的入口在 PR 描述中列明
- [ ] 视图和组件里没有字面量颜色、字号、圆角
- [ ] 新文案已加入 zh-CN 和 en-US 两份 yaml
- [ ] 两套主题都检查过；新增的前景/背景组合对比度 ≥ 4.5:1（大字 ≥ 3:1），并在 PR 中写出数值
- [ ] 可以只用键盘完成本页的主要操作，焦点环可见
- [ ] 测试、构建、clippy 全部通过；没有运行或失败的步骤如实写明
- [ ] PR 描述附上两套主题的截图
