# 配置详解

> 完整配置项参考，适合需要深度定制的用户。普通用户通常只需通过界面操作。

---

## 📁 配置文件结构

```
Data/
├── config.yaml              # 应用配置 (用户可编辑)
├── runtime-config.yaml      # 运行时配置 (自动生成，勿手动修改)
├── profiles/                # 订阅配置目录
│   ├── <name>.yaml          # 单个订阅原始内容
│   └── index.json           # 订阅元数据索引
├── rules/                   # 规则集缓存 (自动更新)
│   ├── direct.yaml
│   ├── proxy.yaml
│   ├── ai.yaml
│   ├── media.yaml
│   └── ad.yaml
├── logs/                    # 日志目录
└── core-session.json        # 核心进程会话信息 (自动管理)
```

---

## ⚙️ config.yaml 完整字段

```yaml
# ────────────────────────────────────────
# 基础设置
# ────────────────────────────────────────
locale: "zh-CN"                    # 界面语言: zh-CN / en-US
theme: "dark"                      # 主题: light / dark / system
auto_check_update: true            # 启动后静默检查更新
auto_update_subscription: false    # 启动后自动更新订阅 (单次)

# ────────────────────────────────────────
# 代理核心设置
# ────────────────────────────────────────
core:
  # 核心二进制路径 (相对便携根目录)
  bin_path: "App/ClashEdge/clash-edge-core.exe"

  # 混合端口 (HTTP/SOCKS5/透明代理共用)
  mixed_port: 7890

  # 允许局域网连接
  allow_lan: true

  # 绑定地址 (0.0.0.0 = 全网卡，127.0.0.1 = 仅本机)
  bind_address: "0.0.0.0"

  # 运行模式: rule / global / direct
  mode: "rule"

  # 日志级别: silent / error / warning / info / debug
  log_level: "error"

  # IPv6 支持
  ipv6: false

  # 进程探测模式: strict / off
  find_process_mode: "strict"

# ────────────────────────────────────────
# 系统代理
# ────────────────────────────────────────
system_proxy:
  enabled: false                   # 是否开启系统代理
  pac_url: ""                      # PAC URL (留空使用默认 PAC)
  bypass_list: []                  # 绕过列表 (域名/IP/CIDR)

# ────────────────────────────────────────
# TUN 模式 (需管理员权限)
# ────────────────────────────────────────
tun:
  enabled: false                   # 是否开启 TUN
  # 网卡名称 (留空自动)
  interface_name: ""
  # MTU (留空自动)
  mtu: 9000
  # 严格路由 (防泄漏)
  strict_route: true
  # 排除路由 (CIDR 列表，如局域网)
  route_exclude_address: []
  # DNS 劫持端口
  dns_hijack_port: 53
  # 自动安装 Wintun 驱动
  auto_install_driver: true

# ────────────────────────────────────────
# DNS 配置
# ────────────────────────────────────────
dns:
  enabled: true
  listen: "0.0.0.0:9053"
  ipv6: false

  # 增强模式: fake-ip / redir-host / off
  enhanced_mode: "fake-ip"
  # Fake-IP 网段
  fake_ip_range: "198.18.0.1/16"
  # Fake-IP 白名单 (不走 Fake-IP)
  fake_ip_filter:
    - "*.lan"
    - "*.local"
    - "*.home.arpa"
    - "localhost.ptlogin2.qq.com"
    - "*.msftconnecttest.com"
    - "*.msftncsi.com"
    - "*.n.n.srv.nintendo.net"

  # 默认 DNS (启动期、直连)
  default_nameserver:
    - "223.5.5.5"
    - "119.29.29.29"

  # 上游 DNS (DoH)
  nameserver:
    - "https://dns.alidns.com/dns-query"
    - "https://doh.pub/dns-query"

  # 回退 DNS
  fallback:
    - "https://dns.google/dns-query"
    - "https://cloudflare-dns.com/dns-query"
  fallback_filter:
    geoip: true
    geoip_code: "CN"
    ipcidr:
      - "240.0.0.0/4"

# ────────────────────────────────────────
# 流量嗅探
# ────────────────────────────────────────
sniffer:
  enabled: true
  sniff:
    HTTP:
      ports: [80, "8080-8880"]
      override_destination: true
    TLS:
      ports: [443, 8443]
    QUIC:
      ports: [443, 8443]
  force_dns_mapping: true
  parse_pure_ip: true

# ────────────────────────────────────────
# GEO 数据
# ────────────────────────────────────────
geodata:
  mode: true                       # 启用内置 Geo
  loader: "standard"               # standard / memconservative
  auto_update: true                # 自动更新
  update_interval: 24              # 小时
  # 自定义源 (留空使用内置镜像)
  geoip_url: ""
  geosite_url: ""
  geox_url: ""

# ────────────────────────────────────────
# 实验性/高级
# ────────────────────────────────────────
advanced:
  # 控制器端口 (0 = 自动分配)
  controller_port: 0
  # 配置热重载间隔 (毫秒)
  reload_interval: 1000
  # 连接统计上报间隔
  stats_interval: 1000

# ────────────────────────────────────────
# 订阅配置 (由界面管理，通常不手动编辑)
# ────────────────────────────────────────
profiles:
  active: "default"                # 当前激活的订阅名称
  items: {}                        # 订阅元数据 (URL、UA、更新时间等)

# ────────────────────────────────────────
# 自定义 Mihomo 配置 (高级，覆盖/合并运行时配置)
# ────────────────────────────────────────
mixin: {}                          # 深度合并到 runtime-config.yaml
```

---

## 🎛️ 界面对应配置映射

| 界面位置 | 配置字段 | 说明 |
|----------|----------|------|
| 设置 → 常规 → 语言 | `locale` | 重启生效 |
| 设置 → 常规 → 主题 | `theme` | 即时生效 |
| 设置 → 常规 → 启动检查更新 | `auto_check_update` | 后台静默 |
| 设置 → 常规 → 启动更新订阅 | `auto_update_subscription` | 单次 |
| 设置 → 代理 → 混合端口 | `core.mixed_port` | 重启核心生效 |
| 设置 → 代理 → 允许局域网 | `core.allow_lan` | 重启核心生效 |
| 设置 → 代理 → 绑定地址 | `core.bind_address` | 重启核心生效 |
| 设置 → 代理 → IPv6 | `core.ipv6` | 重启核心生效 |
| 设置 → 代理 → 运行模式 | `core.mode` | 即时生效 (热重载) |
| 设置 → 代理 → 日志级别 | `core.log_level` | 重启核心生效 |
| 设置 → 系统代理 → 启用 | `system_proxy.enabled` | 即时生效 |
| 设置 → 系统代理 → PAC URL | `system_proxy.pac_url` | 即时生效 |
| 设置 → TUN → 启用 | `tun.enabled` | 需提权，即时生效 |
| 设置 → TUN → 严格路由 | `tun.strict_route` | 重启核心生效 |
| 设置 → TUN → 排除路由 | `tun.route_exclude_address` | 重启核心生效 |
| 设置 → DNS → 增强模式 | `dns.enhanced_mode` | 重启核心生效 |
| 设置 → DNS → Fake-IP 网段 | `dns.fake_ip_range` | 重启核心生效 |
| 设置 → DNS → 上游 DNS | `dns.nameserver` | 重启核心生效 |
| 设置 → 高级 → Geo 自动更新 | `geodata.auto_update` | 后台单次 |
| 设置 → 高级 → 控制器端口 | `advanced.controller_port` | 重启核心生效 |

---

## 🔧 进阶配置技巧

### 1. 自定义规则 (Mixin)

在 `config.yaml` 中添加 `mixin` 字段，可深度合并到运行时配置：

```yaml
mixin:
  # 追加自定义规则 (最高优先级)
  rules:
    - "DOMAIN-SUFFIX,mycompany.internal,DIRECT"
    - "PROCESS-NAME,myapp.exe,扶梯出行"
  
  # 覆盖 DNS
  dns:
    nameserver:
      - "https://doh.mydns.example/dns-query"
  
  # 添加额外代理组
  proxy-groups:
    - name: "公司内网"
      type: select
      proxies: ["DIRECT", "扶梯出行"]
```

> **注意**：`mixin` 会在每次重载配置时合并，优先级高于内置配置。

### 2. 多网卡环境指定出口网卡

```yaml
tun:
  interface_name: "Wintun- ClashEdge"  # 指定网卡名称
  route_exclude_address:
    - "192.168.1.0/24"    # 公司内网
    - "10.10.0.0/16"      # 另一网段
```

### 3. 解决 Fake-IP 兼容性问题

某些应用 (银行、企业内网、游戏) 不兼容 Fake-IP：

```yaml
dns:
  fake_ip_filter:
    - "*.mybank.com"
    - "*.internal.corp"
    - "*.steamcontent.com"
    - "*.battle.net"
  # 或完全禁用增强模式
  enhanced_mode: "redir-host"
```

### 4. 自定义订阅 User-Agent

```yaml
profiles:
  items:
    "我的机场":
      user_agent: "ClashEdge/1.0 (Custom-UA)"
```

或在订阅编辑对话框直接修改。

### 5. 禁用流量嗅探 (性能/兼容性)

```yaml
sniffer:
  enabled: false
```

---

## 🌐 规则集配置

### 内置规则提供者 (自动管理)

```yaml
rule-providers:
  direct:
    type: file
    behavior: classical
    path: "./rules/direct.yaml"
  ad:
    type: file
    behavior: classical
    path: "./rules/ad.yaml"
  ai:
    type: file
    behavior: classical
    path: "./rules/ai.yaml"
  media:
    type: file
    behavior: classical
    path: "./rules/media.yaml"
  proxy:
    type: file
    behavior: classical
    path: "./rules/proxy.yaml"
```

### 规则链 (固定顺序)

```yaml
rules:
  - "GEOSITE,private,DIRECT"
  - "IP-CIDR,127.0.0.0/8,DIRECT,no-resolve"
  - "IP-CIDR,10.0.0.0/8,DIRECT,no-resolve"
  - "IP-CIDR,172.16.0.0/12,DIRECT,no-resolve"
  - "IP-CIDR,192.168.0.0/16,DIRECT,no-resolve"
  - "IP-CIDR,100.64.0.0/10,DIRECT,no-resolve"
  - "RULE-SET,direct,DIRECT"
  - "RULE-SET,ad,REJECT"
  - "GEOSITE,category-ads-all,REJECT"
  - "RULE-SET,ai,人工智能"
  - "RULE-SET,media,影音视听"
  - "RULE-SET,proxy,扶梯出行"
  - "GEOSITE,cn,DIRECT"
  - "GEOIP,CN,DIRECT,no-resolve"
  - "MATCH,扶梯出行"
```

---

## 🔄 热重载机制

ClashEdge 采用**配置哈希比对**决定是否热重载：

1. 用户修改设置 / 切换订阅 / 切换节点
2. `AppController` 事务锁内：校验 → 持久化 → 重生成 `runtime-config.yaml`
3. 计算新配置 SHA256，与上次应用的哈希比对
4. **哈希不同** → PUT `/configs` 热重载 mihomo + 健康检查
5. **哈希相同** → 仅持久化 + 刷新托盘/UI，**跳过热重载**

**触发热重载的操作**：
- 切换订阅 / 更新订阅节点
- 切换代理模式 (Rule/Global/Direct)
- 切换节点 (人工优选组)
- 启用/禁用系统代理、TUN
- 修改 DNS、嗅探、Geo 等核心参数
- 编辑 Mixin 配置

**不触发热重载的操作**：
- 修改语言、主题、UI 设置
- 仅打开设置页未修改
- 订阅元数据更新 (URL、UA、备注)

---

## 📝 配置迁移与备份

### 备份关键文件

```powershell
# 完整备份
Compress-Archive -Path Data/ -DestinationPath "ClashEdge-Backup-$(Get-Date -Format 'yyyyMMdd').zip"

# 仅配置备份
Copy-Item Data/config.yaml, Data/profiles/ -Destination "Backup/" -Recurse
```

### 迁移到新电脑

1. 复制整个 `ClashEdge` 文件夹
2. 新电脑运行 `ClashEdge.exe`
3. 若 TUN 驱动未安装，首次开启 TUN 时自动安装

### 版本升级兼容性

- `config.yaml` 向后兼容，旧字段保留，新字段使用默认值
- 启动时自动运行迁移脚本 (`config/migration.rs`)
- 订阅文件 `profiles/*.yaml` 格式稳定，跨版本可用

---

## 🔍 调试配置

### 查看当前运行时配置

1. 设置 → 高级 → 「查看运行时配置」 (只读)
2. 或直接打开 `Data/runtime-config.yaml`

### 验证配置语法

```powershell
# 使用 mihomo 自带测试
& "App/ClashEdge/clash-edge-core.exe" -t -f "Data/runtime-config.yaml"
```

### 常见配置错误

| 错误现象 | 可能原因 | 解决 |
|----------|----------|------|
| 端口冲突 | `mixed_port`/`controller_port` 被占用 | 修改端口或关闭占用程序 |
| TUN 启动失败 | 无管理员权限 / 驱动未安装 | 以管理员运行 / 重装驱动 |
| DNS 解析失败 | 上游 DoH 不通 / Fake-IP 冲突 | 检查网络 / 调整 `fake_ip_filter` |
| 规则加载失败 | YAML 语法错误 / 文件损坏 | 检查 `runtime-config.yaml` 语法 |

---

> **提示**：大部分配置通过界面修改即可，手动编辑 `config.yaml` 仅用于界面未暴露的高级选项。修改后建议通过界面「重载配置」按钮应用，或重启核心。