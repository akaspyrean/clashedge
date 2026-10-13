# 便携包目录结构与行为

> 说明 ClashEdge Windows 便携版的三分离布局、启动器行为、数据管理。

---

## 📁 目录树

```
ClashEdge/                      # 便携根目录 (用户解压位置)
├── ClashEdge.exe               # C# 根启动器 (入口点)
├── App/                        # 程序文件 (只读推荐)
│   ├── ClashEdge/              # Tauri 应用本体
│   │   ├── ClashEdge.exe       # 内层 Tauri 可执行文件
│   │   ├── sidecar/            # Sidecar 二进制
│   │   │   ├── clash-edge-core.exe   # mihomo 内核 (重命名)
│   │   │   └── wintun.dll            # Wintun 驱动
│   │   ├── resources/          # 静态资源
│   │   │   ├── app.icon.icns
│   │   │   ├── icon.ico
│   │   │   └── i18n/           # 后端 i18n (zh-CN.yaml, en-US.yaml)
│   │   └── ...                 # 其他 Tauri 运行时文件
│   ├── DefaultData/            # 出厂默认数据 (只读模板)
│   │   ├── config.yaml         # 默认应用配置
│   │   ├── Country.mmdb        # MaxMind GeoIP
│   │   ├── GeoIP.dat           # Mihomo GeoIP
│   │   ├── GeoSite.dat         # Mihomo GeoSite
│   │   └── rules/              # 默认规则集 (5 个 .yaml)
│   └── portable.dat            # 便携标识文件 (存在即便携模式)
├── Data/                       # 用户数据 (读写，迁移核心)
│   ├── config.yaml             # 用户应用配置
│   ├── runtime-config.yaml     # 运行时配置 (自动生成)
│   ├── profiles/               # 订阅配置
│   │   ├── *.yaml              # 单订阅原始内容
│   │   └── index.json          # 元数据索引
│   ├── rules/                  # 规则集缓存 (自动更新)
│   │   ├── direct.yaml
│   │   ├── proxy.yaml
│   │   ├── ai.yaml
│   │   ├── media.yaml
│   │   └── ad.yaml
│   ├── logs/                   # 日志文件
│   │   ├── clash_edge.log      # 应用日志
│   │   ├── mihomo-YYYYMMDD.log # 核心日志 (按天轮转)
│   │   └── launcher.log        # 启动器日志
│   └── core-session.json       # 核心进程会话 (PID、路径、时间)
├── Other/                      # 附属文件
│   ├── Help/                   # 帮助文档
│   │   └── README.md
│   └── Licenses/               # 许可证文件
│       ├── GPL-3.0.txt
│       ├── MIT.txt
│       └── ...
└── (用户自建)                  # 用户可自建文件夹
```

---

## 🔑 关键设计原则

### 三分离
| 层级 | 目录 | 特性 | 迁移性 |
|------|------|------|--------|
| **启动器** | 根目录 `ClashEdge.exe` | 单文件、无依赖、C# 编译 | 随包更新 |
| **程序** | `App/ClashEdge/` | Tauri + Sidecar、版本绑定 | 整体替换 |
| **数据** | `Data/` | 用户唯一状态、不含代码 | **完整迁移核心** |

### 便携判定逻辑 (启动器)

```csharp
// 优先级 1：根目录存在 portable.dat
// 优先级 2：根目录存在 clash-edge-core.exe (兼容旧版重命名)
bool isPortable = File.Exists("portable.dat") || File.Exists("clash-edge-core.exe");
```

- **改名/换盘符/移动整个文件夹** 均可自愈，无硬编码路径
- 便携模式下 mihomo 固定解析 `<exe_dir>/App/ClashEdge/clash-edge-core.exe`
- **无 `%APPDATA%` 静默回退**：便携模式严格使用 `Data/`，绝不写用户配置文件夹

---

## 🚀 启动器行为 (`ClashEdge.exe`)

### 职责
1. **环境准备**：设置 `CLASH_EDGE_DATA_DIR` 环境变量指向 `Data/`
2. **进程守护**：拉起内层 `App/ClashEdge/ClashEdge.exe` 并监控
3. **更新应用**：处理便携包更新 (下载、验签、原子替换、回滚)
4. **单实例**：命名 Mutex 防止多开 (同一便携目录仅允许一个实例)
5. **故障恢复**：启动器崩溃/被杀后，下次启动自动恢复中断的更新

### 启动流程
```
用户双击 ClashEdge.exe
    ↓
检测便携模式 (portable.dat 或 clash-edge-core.exe)
    ↓
设置 CLASH_EDGE_DATA_DIR = <根目录>/Data
    ↓
检查更新暂存区 (Data/update-staging/)
    ├─ 有待应用更新 → 应用更新 (见下)
    └─ 无更新 → 继续
    ↓
检查单实例 Mutex (Global\ClashEdge_<路径哈希>)
    ├─ 已运行 → 激活现有窗口 (WM_SHOWME) → 退出
    └─ 未运行 → 继续
    ↓
启动内层 App/ClashEdge/ClashEdge.exe (CREATE_NO_WINDOW)
    ↓
监控子进程退出码
    ├─ 正常退出 (0) → 清理 → 退出
    ├─ 重启请求 (特定码) → 重新拉起内层
    └─ 异常退出 → 记录日志 → 退出
```

### 更新应用流程 (原子、可回滚)

```
Data/update-staging/           # 暂存区 (由内层下载、验签后写入)
├── portable-manifest.json     # 更新清单 (含版本、文件列表、SHA256)
├── portable-manifest.json.minisig  # minisign 签名
├── ClashEdge.exe              # 新启动器
├── App/                       # 新程序文件 (完整镜像)
└── Data/DefaultData/          # 新默认数据 (可选)
```

**应用步骤** (启动器 `ApplyPendingUpdate`)：
1. 读取 `portable-manifest.json`，验签 (内置 minisign 公钥)
2. 逐文件校验 SHA256
3. **备份当前程序**：`App/` → `App.backup.<timestamp>/` (Directory.Move)
4. **原子替换**：`Data/update-staging/App/` → `App/` (Directory.Move)
5. 替换根启动器：`Data/update-staging/ClashEdge.exe` → `ClashEdge.exe` (文件替换)
6. 写入 `update-applied.json` 标记成功
7. **清理暂存区**：删除 `Data/update-staging/`
8. 启动新版内层程序

**回滚机制**：
- 任意步骤失败 → 从 `App.backup.*` 恢复 → 保留暂存区 → 下次启动重试
- `App.backup.*` 保留最近 3 个，超额自动清理

### 运行中应用更新 (关键场景)

**场景**：用户最小化到托盘 → 后台下载更新完成 → 用户再次双击 `ClashEdge.exe` 唤醒窗口

**行为**：
1. 启动器检测到内层应用**正在运行** (Mutex 存在 + 进程存活)
2. **跳过应用更新**，仅发送 `WM_SHOWME` 激活窗口
3. **保留 `Data/update-staging/` 完整不变**
4. 用户真正退出应用 (托盘菜单"退出") → 下次启动时应用更新

> **设计理由**：避免 `Directory.Move(App, App.backup)` 因内层 exe 正在运行而失败，导致已验签更新包被静默丢弃。

---

## 📦 Data/ 目录详解

### config.yaml (用户配置)
- 界面修改的所有设置持久化位置
- 启动时加载，运行时内存驻留，修改即时写入 (原子写入：临时文件 + sync + rename)
- **不含敏感明文**：订阅 URL 在 `profiles/*.yaml` 头注释中 (见下)

### runtime-config.yaml (运行时配置)
- **自动生成，勿手动修改**
- 由 `AppConfig` + 激活 `Profile` 合成
- mihomo 实际加载的配置文件
- 每次热重载前计算 SHA256，**哈希未变则跳过热重载**

### profiles/ (订阅存储)
```
profiles/
├── 我的机场.yaml              # 订阅原始内容 (含 proxies 等)
├── 备用机场.yaml
└── index.json                 # 元数据索引
```

**index.json 结构**：
```json
{
  "active": "我的机场",
  "items": {
    "我的机场": {
      "url": "https://...",
      "user_agent": "ClashEdge/1.0 (mihomo)",
      "updated_at": "2026-10-13T10:30:00Z",
      "node_count": 42,
      "subscription_userinfo": "upload=123; download=456; total=1073741824; expire=1730000000"
    }
  }
}
```

> **安全提示**：订阅 URL (含 token) 以明文保存在 `profiles/*.yaml` 头注释。便携迁移到他机器时需重新输入订阅地址。

### rules/ (规则集缓存)
- 5 个规则文件：`direct.yaml`、`proxy.yaml`、`ai.yaml`、`media.yaml`、`ad.yaml`
- 由规则更新器 (`geodata/rules.rs`) 每日自动更新
- 更新流程：下载签名清单 → minisign 验签 → 下载文件 → SHA256 校验 → 事务替换
- **类型均为 `file`**，mihomo 不直接联网拉取规则

### logs/ (日志)
| 文件 | 轮转策略 | 保留 |
|------|----------|------|
| `clash_edge.log` | 2 MiB × 5 份 | 5 个文件 |
| `mihomo-YYYYMMDD.log` | 按天 + 大小 | 30 天 |
| `launcher.log` | 1 MiB × 3 份 | 3 个文件 |

- Release 默认 `Info` 级别，`CLASH_EDGE_LOG=debug` 环境变量开启 `Debug`
- 设置 → 关于 → 「导出诊断包」可打包最近日志

### core-session.json (核心会话)
```json
{
  "pid": 12345,
  "image_path": "D:\\ClashEdge\\App\\ClashEdge\\clash-edge-core.exe",
  "started_at": "2026-10-13T10:30:00.123Z"
}
```
- 核心启动成功后写入，停止确认后删除
- **孤儿回收**：下次启动前读取 → 校验映像路径匹配 → 终止残留进程 → 删除文件

---

## 🔄 数据迁移与备份

### 完整迁移 (换电脑/换盘符/重装系统)
```powershell
# 1. 复制整个 ClashEdge 文件夹到新位置
# 2. 新位置运行 ClashEdge.exe
# 3. 若 TUN 驱动未安装，首次开启 TUN 时自动安装 (需管理员)
```

### 仅备份配置
```powershell
# 关键文件
Data/config.yaml
Data/profiles/
Data/rules/          # 可选，自动更新会恢复
```

### 版本升级兼容
- `config.yaml`：向后兼容，迁移脚本自动补全新字段
- `profiles/*.yaml`：格式稳定，跨版本可用
- `runtime-config.yaml`：每次启动重新生成，无需迁移
- `Data/DefaultData/`：随程序更新，用户数据不受影响

---

## 🛡️ 权限与安全

| 文件/目录 | 权限建议 | 说明 |
|-----------|----------|------|
| `ClashEdge.exe` (根) | 读/执行 | 启动器，需写入自身更新 |
| `App/` | 读/执行 | 程序文件，**建议设为只读** (防篡改) |
| `App/DefaultData/` | 只读 | 模板文件 |
| `Data/` | 读/写 | **用户数据，必须可写** |
| `Data/logs/` | 读/写/创建 | 日志轮转需创建文件 |
| `Data/update-staging/` | 读/写/创建/删除 | 更新暂存区 |

### 杀毒软件白名单建议
将以下加入信任/排除扫描：
- `ClashEdge.exe` (根启动器)
- `App/ClashEdge/ClashEdge.exe` (内层 Tauri)
- `App/ClashEdge/clash-edge-core.exe` (mihomo 核心)
- `App/ClashEdge/wintun.dll` (Wintun 驱动)
- `Data/` 目录 (频繁读写，排除扫描可提升性能)

---

## 🔧 故障排查

| 现象 | 排查步骤 |
|------|----------|
| 双击无反应 | 1. 杀毒软件拦截 → 白名单 2. 以管理员运行测试 3. 查看 `Data/logs/launcher.log` |
| 提示"不是有效应用" | 1. 文件损坏 → 重新下载校验 SHA256 2. 架构不匹配 (ARM64 设备) |
| 更新后无法启动 | 1. 查看 `Data/logs/launcher.log` 2. 检查 `App.backup.*` 是否存在 → 手动恢复 3. 删除 `Data/update-staging/` 重试 |
| 配置丢失 | 1. 确认 `Data/config.yaml` 存在 2. 检查是否误删 `Data/` 3. 便携模式下无 `%APPDATA%` 回退 |
| TUN 无法开启 | 1. 以管理员运行 2. 企业策略禁止驱动安装 3. `wintun.dll` 签名验证失败 |

---

## 📋 打包清单 (发布时)

`build-portable.ps1` 生成的 `ClashEdge-portable-win64.zip` 包含：

```
ClashEdge-portable-win64.zip
├── ClashEdge.exe                          # 启动器 (已 Authenticode 签名)
├── App/
│   ├── ClashEdge/ClashEdge.exe            # 内层 Tauri (已签名)
│   ├── ClashEdge/sidecar/clash-edge-core.exe  # mihomo (MetaCubeX 签名)
│   ├── ClashEdge/sidecar/wintun.dll       # Wintun (WireGuard 签名)
│   ├── ClashEdge/resources/...            # 资源文件
│   ├── DefaultData/                       # 默认数据 (GeoIP、规则、配置)
│   └── portable.dat                       # 便携标识
├── Other/
│   ├── Help/README.md
│   └── Licenses/*.txt
└── (无 Data/ 目录，首次运行自动创建)
```

**校验文件** (同目录)：
- `ClashEdge-portable-win64.zip.sha256` - SHA256 哈希
- `portable-manifest.json` - 更新清单 (含版本、文件列表、SHA256)
- `portable-manifest.json.minisig` - minisign 签名

---

> **核心原则**：**程序可替换、数据可迁移、更新可回滚、便携无依赖**。