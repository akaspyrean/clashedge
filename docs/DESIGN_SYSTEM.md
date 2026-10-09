# ClashEdge Design System · Clear Edge v2

> **版本 2.0** · 适用 ClashEdge Windows 客户端（Tauri 2 + Vue 3 + Element Plus）
> **v1.0（Hybrid / Quiet Power）已被本版本取代。** v1.0 中「圆角 ≤ 12」「只用 ease 曲线」「禁止缩放动画」等规则不再适用。
>
> 单一事实来源：[`design/tokens.json`](../design/tokens.json)。
> 运行 `npm run tokens`（在 `apps/windows` 下）生成 `src/styles/tokens.css` 与 Android 的 `Tokens.kt`，**不要手改生成文件**。
> 开发任务拆分与验收见 [`docs/redesign/AI_DEV_INSTRUCTIONS.md`](redesign/AI_DEV_INSTRUCTIONS.md)。

---

## 1. 原则

- **清晰、克制、可读。** 层级靠留白、字号与底色，不靠装饰。
- **状态不只靠颜色。** 色点必须配形状（圆 / 菱形 / 横杠 / 转圈 / 空心圆）和文字。
- **数值等宽。** 速率、延迟、流量、端口、计数一律 `font-variant-numeric: tabular-nums`；端口、IP、URL、日志、规则用 `--ce-font-mono`。
- **一个视图最多一个 primary 按钮。** 品牌蓝只做填充，文字与图标用 `accent-fg`。
- **不动功能，只改呈现。** 代理组（扶梯出行 · 人工智能 · 影音视听 · 人工优选 · 自动优选，另有隐藏的 GLOBAL）的名称、数量、类型、顺序不可改动。

## 2. 颜色

所有颜色都是 `--ce-*` Token，随 `html[data-theme="light|dark"]` 切换。完整数值见 `design/tokens.json`。

| 角色 | Token | 用途 |
|---|---|---|
| 页面 / 卡片 / 弹层 | `bg-page` · `bg-surface` · `bg-elevated` | 页面底 / 卡片与侧栏 / 菜单与对话框 |
| 悬停 / 按下 | `fill-hover` · `fill-soft` | 悬停底 / 按下底、分段控件轨道、搜索框底 |
| 描边 / 分割 | `border` · `divider` | 1px 描边 / 组内分割线 |
| 文字 | `text-primary` · `-secondary` · `-tertiary` · `-disabled` | 主 / 次 / 三级（表头、占位）/ 禁用 |
| 品牌 | `accent-bg`(+`-hover`/`-press`) · `accent-fg` · `accent-soft` · `on-accent` | **accent-bg 只用作填充**（主按钮、开关开启、选中单选，配白字 5.01:1）；**accent-fg 只用作文字或图标**（链接、选中项文字）；accent-soft 选中浅底 |
| 语义 | `success` · `warning` · `danger` · `pending` 各 `-solid` / `-fg` / `-soft` | **`-solid` 只能用于色点、图标、图表线条，不能用于文字**；彩色文字一律用 `-fg`；`-soft` 为标签与提示的底 |
| 危险填充 | `danger-bg` | 危险按钮填充，配白字 5.22:1 |
| 图表 | `chart-down` · `chart-up` · `chart-grid` | 下载 / 上传 / 网格线 |
| 其他 | `switch-off` · `toast-bg` · `toast-text` · `scrim` · `idle` | 开关关闭轨道 / Toast / 遮罩 / 未运行 |

对比度：accent-bg 上白字 5.01:1；danger-bg 上白字 5.22:1；浅色 accent-fg/surface 6.36:1；深色 accent-fg/surface 5.81:1、/accent-soft 5.15:1。新增前景/背景组合必须 ≥ 4.5:1（大字 ≥ 3:1）。

## 3. 字体与字号

`--ce-font-sans`（Segoe UI Variable → YaHei UI → 系统）、`--ce-font-mono`（JetBrains Mono，OFL，随包附带于 `src/assets/fonts/`）。CSP 只允许 `font-src 'self' data:`，**禁止使用 Google Fonts 或任何 CDN**。

| Token（桌面 字号/行高/字重） | 用途 |
|---|---|
| `type-display` 40/48/700 | 首页速率等关键数值（只用于数值） |
| `type-title-1` 28/36/600 | 页面标题 `.page-title` |
| `type-title-2` 20/28/600 | 区块标题、对话框标题 |
| `type-title-3` 16/24/600 | 卡片标题 |
| `type-body` 14/22/400（按钮 500） | 正文 |
| `type-callout` 13/20/400 | 辅助说明 |
| `type-caption` 12/16/400·500 | 标签、表头、时间 |
| `type-mono` 13/20/400 | 日志、规则、IP |

用法：`font: var(--ce-type-title-3);`，不要写字号字面量。

## 4. 形状、间距、阴影、动效、尺寸

- **圆角**：`radius-xs 6 · sm 8 · md 12 · lg 16 · xl 20 · full 999`。按钮、输入框 md；卡片 lg（节点格 14）；对话框 xl；延迟标签 sm。
- **间距**：`space-1…16`（4 / 8 / 12 / 16 / 20 / 24 / 32 / 40 / 48 / 64）。
- **阴影**：`shadow-e1`（分段控件选中段）· `e2`（菜单、Toast）· `e3`（对话框）。**卡片默认不加阴影**：1px `border` + surface 底。
- **动效**：`ease-standard / enter / exit`；`ease-spring` 仅用于开关拇指和主连接按钮；`dur-instant 80 · fast 140 · base 220 · slow 320`。保留 `prefers-reduced-motion` 全局降级。
- **尺寸**：`control-h 36`（`-sm` 28、`-lg` 44）· `row-h 44` · `icon 20`（描边 1.75）· `sidebar-w 232`（收起 64）。

## 5. 延迟分级（全局统一，设置中可调阈值）

| 档位 | 条件 | 形状 | 颜色组 | 显示 |
|---|---|---|---|---|
| 良好 | ≤ 200 ms | 圆 | success | `86 ms` |
| 一般 | 201–500 ms | 菱形 | warning | `320 ms` |
| 较差 | > 500 ms | 横杠 | danger | `860 ms` |
| 超时 | 超时或失败 | 无 | `fill-soft` 底 + secondary 文字 | `超时` |
| 测速中 | — | 转圈 | pending | `测速中` |
| 未测 | 无数据 | 无 | 1px 虚线描边 + tertiary 文字 | `– ms` |

读屏文案：「延迟 86 毫秒，良好」。实现：`src/utils/latency.ts` + `components/ui/CeLatencyTag.vue`。

## 6. 组件

自有组件在 `src/components/ui/`，都只引用 `--ce-*`：

| 组件 | 说明 |
|---|---|
| `CeSegmented` | 分段控件，`role="radiogroup"`，方向键 / Home / End 切换；用于代理模式、主题 |
| `CeLatencyTag` | 延迟标签，可点击（单击重新测该节点） |
| `CeNodeTile` | 节点格：覆盖整格的选择按钮 + 独立的延迟标签按钮（不嵌套按钮）；选中 = accent 描边 + 右上角勾 |
| `CeStatusCore` | 主连接按钮（直径 96）+ 状态文字；停止 / 启动中 / 运行中 / 异常 |
| `CeTrafficChart` | 60 秒流量图，手写 SVG，单 Y 轴，十字线 + Tooltip，「查看数据表」 |
| `CeSparkline` | 首页迷你曲线 |
| `StatusPill` | 状态胶囊：色点形状 + 颜色 + 文字 |

Element Plus 通过 `styles.css` 桥接：彩色**文字**位置取 `*-fg`；**填充**位置（主 / 危险按钮、开关、选中单选与复选）指回 `accent-bg` / `danger-bg` 并配白字。按钮按下 `scale(.98)`；焦点环统一 `2px solid var(--ce-accent-bg)`、偏移 2px。

- **卡片**：surface 底 + 1px border + lg 圆角 + 内边距 20–24（全局 `.ce-card`）。**卡片里不再嵌套卡片**，内部分组用 `divider`。
- **危险操作**走 `useConfirm(..., { danger: true })`：确认按钮写明具体动作（如「删除订阅」），默认焦点在「取消」，按钮顺序 [取消][删除…]。
- **Toast**（ElMessage）：高 44，`toast-bg`，e2；普通 3s，带操作 5s，错误 6s。
- **图标**：`lucide-vue-next`，尺寸 20，`stroke-width` 1.75，只用线性图标。不使用 emoji、毛玻璃（`backdrop-filter`）、渐变、装饰性大阴影。

## 7. 信息架构

侧栏顺序：**首页 / 代理 / 订阅 / 规则 / 连接 / 日志 / 设置**。更名只改 i18n 的值，**不改路由路径和 i18n 键名**（避免影响托盘和测试）。

| 导航 | 路由 | 视图 |
|---|---|---|
| 首页 | `/dashboard` | `DashboardView`：状态核心、实时流量、快捷切换、当前订阅 |
| 代理 | `/proxies` | `ProxiesView`：策略组列表 + 节点格 |
| 订阅 | `/profiles` | `ProfilesView`：订阅卡片网格 |
| 规则 | `/rules` | `RulesView`：只读 |
| 连接 | `/connections` | `ConnectionsView`：流量图 + 活动连接 |
| 日志 | `/logs` | `LogsView`：级别筛选 + 正则搜索 |
| 设置 | `/settings` | `SettingsView`：单页分组 + 搜索 |

## 8. 开发规则

1. 视图和组件的样式里**只能引用 `--ce-*` Token 和布局属性**，禁止 HEX、`rgb()`、字号字面量、圆角字面量（`0`、`50%`、`999px` 除外）。CI 会检查（见 §9）。
2. 新文案同时加进 `src-tauri/resources/i18n/zh-CN.yaml` 和 `en-US.yaml`（托盘与前端共用）。
3. 所有可交互元素用原生 `<button>` / `<a>` / `<input>` 或 EP 组件；纯图标按钮必须有 `aria-label`。
4. 改视觉只改 `design/tokens.json` 再运行 `npm run tokens`，不要在视图里临时调色。
5. 行为变化时同步更新 `*.spec.ts`，不要删测试来让 CI 变绿。

## 9. 守护（`scripts/ci/quality.ps1`）

- **token 产物一致**：运行 `npm run tokens` 后，`tokens.css` 与 `Tokens.kt` 必须与运行前完全相同。
- **无颜色字面量**：`apps/windows/src` 下的 `views/`、`components/`、`App.vue`（不含 `*.spec.ts`）出现 HEX 或 `rgb()` / `rgba()` 即失败。
