# ClashEdge Hybrid Design System

> 版本 1.0 · 适用 ClashEdge 桌面端（Tauri + Element Plus）
> Token 实现：`apps/windows/src/styles.css`（单一事实来源）

---

## 0. 品牌性格

> **Quiet Power** —— 安静的力量。

视觉气质：**轻、静、清晰、克制、可靠、有生命力**。

界面不抢戏。层级靠留白与明度海拔表达，不靠装饰；信息密度服务于代理管理场景，不追求"科技感"表演。

---

## 1. 设计来源：吸收，不复刻

| 来源 | 吸收什么 | 明确不吸收什么 |
|------|----------|----------------|
| **Flyme 3** | 轻盈、留白、内容优先、低装饰、干净的列表节奏 | 拟物残留、强彩色分区 |
| **Apple HIG** | 排版纪律、4pt 网格间距、克制的交互反馈、动效时长约束、深色海拔分层 | 玻璃拟态、毛玻璃材质、 vibrancy |
| **ClashEdge 自有** | Gray Blue 中性色系统 + Natural Semantic Colors（自然语义色） | — |

**融合规则**：冲突时 Apple 的排版与间距纪律 > Flyme 的轻盈气质 > 组件库默认值。

### 反模式（出现即回归）

```text
✗ 科技 Dashboard 风        ✗ AI 紫蓝渐变
✗ 卡片墙（卡片套卡片）      ✗ 玻璃拟态 / backdrop-blur
✗ 高饱和状态灯             ✗ 圆角大面包（radius > 16px）
✗ 强阴影                   ✗ 炫技动画（> 300ms 的装饰动效）
✗ 像素级复刻任何厂商 UI
```

---

## 2. 无障碍基线：WCAG AA

所有正文/控件文本对其背景 **≥ 4.5:1**；大号文本（≥18.66px bold 或 24px）**≥ 3:1**。
当前 v1.0 色板实测（相对亮度法）：

| 前景 / 背景 | 对比度 | 达标 |
|-------------|--------|------|
| 深色 text-primary `#F5F6F7` / surface `#191C21` | 15.79 | AAA |
| 深色 text-secondary `#B9BFC7` / surface | 9.22 | AAA |
| 深色 text-tertiary `#858D97` / surface | 5.09 | AA |
| 深色 accent `#4E8FFF` / surface（链接文字） | 5.45 | AA |
| 深色主按钮：深字 `#101214` / accent 底 `#4E8FFF` | **5.99** | AA |
| 浅色 text-primary `#1C1F24` / white | 16.52 | AAA |
| 浅色 text-tertiary `#667085` / white | 4.97 | AA |
| 浅色主按钮：white / accent `#2A62CC` | 5.65 | AA |
| 语义色文字（error/done/approval）/ surface | 8.37–10.15 | AAA |

**铁律**：
1. 深色主题主按钮文字用 `--on-accent-strong`（近黑），**禁止改白字**（仅 3.14:1，不达标）。
2. 新增任何前景/背景组合必须先跑对比度计算再合入。
3. 语义色只用于语义（错误/成功/警告），不做装饰。

---

## 3. 色彩系统

### 3.1 Gray Blue 中性色（海拔分层）

深色主题按"海拔"递进，相邻层亮度差 ≥ 4%，让表面自己浮起来：

```text
bg-app     #0E1013   应用底
bg-sidebar #131519   导航（略高于底）
bg-surface #191C21   卡片 / 弹层
bg-raised  #23272E   卡内嵌块 / hover 面
bg-soft    #262B33   交互反馈底
```

浅色主题镜像同理（app 灰底 → 白卡面 → 灰 raised）。

**用法**：分层优先级 表面底色 > 间距 > 描边。描边只做"确认边界"，不做主要分层手段——`--card-border` 必须比 `--border-subtle` 更弱。

### 3.2 强调色 Accent

| 主题 | 值 | 性格 |
|------|-----|------|
| 深 | `#4E8FFF` | 干净自信的蓝，向 iOS systemBlue(dark) 靠拢 |
| 浅 | `#2A62CC` | 同色相加深保证对比 |

**用量纪律**：一屏内 accent 只出现在「当前选中 / 主操作 / 链接」三种位置。大面积重复出现即为滥用。

### 3.3 自然语义色（Natural Semantic Colors）

| 语义 | 深 | 含义锚点 |
|------|-----|---------|
| error | `#FF9A93` | 失败、危险操作（珊瑚红，非纯红） |
| done/success | `#7EDB99` | 成功、运行中（自然绿） |
| approval/warning | `#F2B866` | 需注意、待确认（琥珀） |

深色下用粉彩明度（作为文字可读），浅色下用对应加深的墨色调（见 token）。状态表达优先用文字 + 小圆点，禁用高饱和大色块灯。

---

## 4. 排版（Apple 纪律）

字体栈：`-apple-system, "SF Pro Text", "PingFang SC", "Segoe UI", "Microsoft YaHei"`。

| 层级 | 字号/字重 | 用途 |
|------|-----------|------|
| page-title | 17px / 500 / letter-spacing 0.2px | 页面标题（轻，不做重锤） |
| card-title | 14px / 500 | 卡片标题 |
| body | 14px / 400 | 正文 |
| secondary | 13px / 400 | 辅助说明 |
| caption | 12px / 400 | 标签、hint、数字（tabular-nums） |
| stat-value | 18px / 600 | 唯一允许的大数值强调 |

**规则**：同屏字重不超过两档（400/500，数值除外）；CJK 不使用 italic；层级靠字号与灰阶，不靠加粗堆叠。

## 5. 间距（4pt 网格）

```text
--space-1: 4px   图标与文字
--space-2: 8px   相关元素间
--space-3: 12px  卡片内组间距
--space-4: 16px  卡片间距 / 页面段落
--space-5: 20px  卡片内边距基准
--space-6: 24px  页面左右留白
```

页面内容最大宽度 1080px；宁可留白，不填满。

## 6. 形状

```text
--r-sm: 8px    按钮 / 输入框 / 菜单项
--r-md: 12px   卡片 / 弹窗
```

上限即 12px，禁止"大面包"。阴影默认 none；弹窗允许 `0 8px 24px rgba(0,0,0,.24)` 一档，其余一律描边+底色分层。

## 7. 动效纪律

```text
--dur-fast: 150ms   hover / 颜色过渡
--dur-base: 200ms   展开收起
easing: ease（唯一缓动）
```

只做透明度与颜色的微过渡；无位移弹跳、无缩放动画、遵守 `prefers-reduced-motion`。

---

## 8. 组件要点

- **侧边导航**：激活态 = 淡染底 + accent 文字，单一强调；图标栏窄窗模式仅在 < 860px 出现。
- **按钮**：primary 每视图最多一个；次操作一律 default；危险操作 danger-plain + 二次确认。
- **空状态**：插画统一收敛 128px，一句话说明 + 一个行动入口。
- **表单**：label 右对齐列宽 150px，控件宽度收敛（select 220px / input ≤ 300px），保存按钮固定在表单尾。
- **反馈**：操作结果一律 ElMessage；破坏性操作 ElMessageBox 确认。

## 9. 维护流程

1. 改视觉 = 改 `styles.css` token，禁止在视图组件里写死色值/字号/圆角。
2. 视图 scoped style 只允许引用 token 与布局属性。
3. 新增颜色组合：先算 WCAG（脚本见 §2），写入本文档表格，再改代码。
4. 本文档与 `styles.css` 头部注释保持版本同步。
