<div align="center">

# ClashEdge

**轻量、便携、开箱即用的 Windows Mihomo 客户端**

订阅负责节点，ClashEdge 负责分流与策略。

![Windows](https://img.shields.io/badge/Windows-10%20%2F%2011-4F6D7A?style=flat-square)
![Version](https://img.shields.io/badge/version-0.8.5-66856A?style=flat-square)
![Portable](https://img.shields.io/badge/Portable-Ready-7D8B74?style=flat-square)
![License](https://img.shields.io/badge/license-MIT-A98652?style=flat-square)

[下载](../../releases) · [规则库](https://github.com/akaspyrean/external)

</div>

---

## 界面

<table>
<tr>
<td width="50%">
<img src="docs/images/clashedge-001.webp" alt="概览">
</td>
<td width="50%">
<img src="docs/images/clashedge-002.webp" alt="代理">
</td>
</tr>
<tr>
<td align="center"><b>概览</b></td>
<td align="center"><b>代理</b></td>
</tr>
<tr>
<td>
<img src="docs/images/clashedge-003.webp" alt="配置">
</td>
<td>
<img src="docs/images/clashedge-004.webp" alt="设置">
</td>
</tr>
<tr>
<td align="center"><b>配置</b></td>
<td align="center"><b>设置</b></td>
</tr>
</table>

---

## 开箱即用

<table>
<tr>
<td align="center" width="20%"><b>📦 便携</b><br><sub>解压即用</sub></td>
<td align="center" width="20%"><b>🧭 分流</b><br><sub>规则预置</sub></td>
<td align="center" width="20%"><b>⚡ 优选</b><br><sub>自动测速</sub></td>
<td align="center" width="20%"><b>🛡️ 拦截</b><br><sub>广告过滤</sub></td>
<td align="center" width="20%"><b>🎛️ 可控</b><br><sub>支持自定义</sub></td>
</tr>
</table>

<br>

|        |                        |
| ------ | ---------------------- |
| **节点** | 导入订阅即可                 |
| **模式** | Rule / Global / Direct |
| **策略** | 人工优选 / 自动优选            |
| **网络** | 系统代理 / TUN             |
| **管理** | 订阅更新 / 配置编辑 / 热重载      |
| **状态** | 延迟 / 流量 / 连接 / 日志      |
| **界面** | 中文 / English · 浅色 / 深色 |

---

## 智能分流

```mermaid
flowchart LR
    A((网络请求))

    A --> AD[广告]
    A --> CN[国内]
    A --> AI[AI]
    A --> M[影音]
    A --> P[其他]

    AD --> X[REJECT]
    CN --> D[DIRECT]

    AI --> S1[人工智能]
    M --> S2[影音视听]
    P --> S3[扶梯出行]

    S1 --> U[人工优选]
    S1 --> T[自动优选]

    S2 --> U
    S2 --> T

    S3 --> U
    S3 --> T

    U --> N((订阅节点))
    T --> N
```

### 内置规则

<table>
<tr>
<td align="center"><b>🛡️ 广告</b><br><code>ad.yaml</code><br>↓<br><b>REJECT</b></td>
<td align="center"><b>🇨🇳 直连</b><br><code>direct.yaml</code><br>↓<br><b>DIRECT</b></td>
<td align="center"><b>✦ AI</b><br><code>ai.yaml</code><br>↓<br><b>人工智能</b></td>
<td align="center"><b>▶ 影音</b><br><code>media.yaml</code><br>↓<br><b>影音视听</b></td>
<td align="center"><b>↗ 代理</b><br><code>proxy.yaml</code><br>↓<br><b>扶梯出行</b></td>
</tr>
</table>

规则来自 [External](https://github.com/akaspyrean/external)，随程序提供并支持更新。

---

## 两种节点选择

```mermaid
flowchart LR
    A[AI / 影音 / 代理]

    A --> M[人工优选]
    A --> T[自动优选]

    M --> M1[手动选择节点]

    T --> T1[自动测速]
    T1 --> T2[选择低延迟节点]
```

<table>
<tr>
<td width="50%" align="center">
<h3>人工优选</h3>
手动决定使用哪个节点
</td>
<td width="50%" align="center">
<h3>自动优选</h3>
自动测速并选择低延迟节点
</td>
</tr>
</table>

---

## 三种模式

```mermaid
flowchart LR
    A{运行模式}

    A --> R[Rule]
    A --> G[Global]
    A --> D[Direct]

    R --> R1[按规则智能分流]
    G --> G1[全部使用指定代理]
    D --> D1[全部直接连接]
```

日常使用选择 **Rule** 即可。

---

## 30 秒开始

```mermaid
flowchart LR
    A[下载 Portable] --> B[解压]
    B --> C[启动 ClashEdge]
    C --> D[添加订阅]
    D --> E[启动核心]
    E --> F((开始使用))
```

从 [Releases](../../releases) 下载：

```text
ClashEdge-portable-<version>-win64.zip
```

解压后运行：

```text
ClashEdge.exe
```

支持：

**中文路径 · 空格路径 · 更换目录 · 更换盘符 · 整体迁移**

---

## 内置，但不锁死

```mermaid
flowchart LR
    A[ClashEdge 默认配置]

    A --> R[内置规则]
    A --> G[内置策略组]
    A --> S[默认网络设置]

    R --> C[自定义配置]
    G --> C
    S --> C

    C --> M((你的 Mihomo))
```

默认配置用于开箱即用。

需要更细的控制时，可以自行调整：

`规则` · `Rule Provider` · `代理组` · `DNS` · `TUN` · `Mihomo 配置`

---

<div align="center">

### ClashEdge

**下载 · 解压 · 添加订阅 · 使用**

[Releases](../../releases) · [External Rules](https://github.com/akaspyrean/external)

<sub>MIT License · 仅用于网络配置、代理管理与技术研究</sub>

</div>
