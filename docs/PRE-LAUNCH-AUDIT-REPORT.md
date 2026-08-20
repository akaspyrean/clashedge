# ClashEdge 0.8.5 上线前审核报告

日期：2026-08-20
审核范围：`tauri-scaffold/src-tauri`（Rust 后端）+ `tauri-scaffold/src`（Vue3 前端）+ 上线配置（`tauri.conf.json` / `capabilities/default.json` / 构建链路）。
方法：双路并行静态审核（后端安全面 / 前端安全与 BUG 面）+ 关键高危项人工代码核实（secret 处理、provider path 透传、SSRF、导航锁、CSP、权限最小化、初始化链路、store 错误处理）。

---

## 1. 总体结论

**基础安全做得扎实**：路径穿越防护（`sanitize_profile_name`，`util/paths.rs:345`）完整覆盖 `/ \ : 保留名 控制符`；mihomo 启动参数、注册表写入、自启动值均无用户输入拼接，**无命令注入面**；TLS 校验默认开启（rustls-tls-webpki-roots）；前端全量 `{{ }}` 文本插值，**无 `v-html` / `eval` / `innerHTML` / DOM clobbering / 动态组件注入**；命令名全为硬编码字面量。

**存在 2 项高危、5 项中危**。核心短板是架构性的：**IPC 攻击面过大且缺少 WebView 纵深防御**，以及**运行时可把控制器密钥打回已知默认值**。另有若干 UI 状态一致性 BUG 与双击竞态。

**结论：不建议直接以当前状态发布。** 优先修复高危项（H1、H2）后可作为发布候选；中危项至少处理 C1/C2/C3。

---

## 2. 高危（必须修复）

### H1【高】运行中命令可重新引入已知默认/空控制器密钥 → 本机任意进程可接管代理
- 位置：`commands/config.rs:17-38`（update/reset/import）→ `config/persistence.rs:88-109`（`update_config` / `reset_config` / `import_config` 直接落盘）→ `config/model.rs:202-208`（默认占位密钥 `"clash-edge-secret"`）→ `core/config.rs:139`（secret 写入 runtime-config）→ 热重载生效。
- 已核实：`reset_config` → `Config::default()` → secret 落为公开已知的 `"clash-edge-secret"` 并立即 `reload_running_core` 生效；`import_config` 导入的 YAML 缺 `secret` 字段时同样落占位值；`update_config` 收到空/缺 secret 的 JSON 亦然。密钥轮换仅在启动 `init()` 时执行（`persistence.rs:39-59`），运行中三条路径均绕过。
- 影响：重置/导入配置后，本机任意进程（含被 XSS 的 WebView、恶意软件）可用已知密钥连接 `127.0.0.1:9090` 接管 mihomo 控制器（切代理、看/断连接）。
- 修复：`update_config` / `import_config` / `reset_config` 落盘前统一校验：secret 为空、等于占位符或历史遗留值（`clash-edge-secret` / `clash-f-win-secret`）时强制 `generate_random_secret()`；补 3 条单元测试。

### H2【高】WebView 任意脚本 = 完全本地接管（纵深防御缺失组合）
- 位置：`main.rs:239-295`（30+ 命令全量暴露）＋ `tauri.conf.json:13`（`withGlobalTauri: true`）＋ `capabilities/default.json:23`（`fs:allow-read-text-file` 覆盖 `$HOME/**`）＋ 无导航锁定（全仓库无 `on_navigation` / `NavigationStarted`）。
- 已核实：`withGlobalTauri` 暴露全局 `window.__TAURI__`（前端全部用显式 import，无需此全局）；`fs:allow-read-text-file` 允许读取主目录任意文本（`.ssh` / `.env` / 浏览器凭据）；当前前端虽无 XSS 向量，但任一依赖漏洞/未来改动即可触发。
- 影响：XSS 出现时攻击者可读任意用户文本文件、窃取配置 secret 与节点密码、发起 SSRF、写注册表（自启/系统代理）、改 TUN。
- 修复：① `withGlobalTauri: false`；② 移除 `$HOME/**` 读权限，仅保留 `$DESKTOP/**`/`$DOCUMENT/**`/`$DOWNLOAD/**`（配置导入用）；③ 加 `NavigationStarted` 处理器，仅放行应用自身 origin（`tauri://localhost` 等），其余 `prevent_navigation`；④ 收紧 CSP（见 C1）。

---

## 3. 中危（建议发布前修复）

| # | 风险 | 位置 | 修复建议 |
|---|------|------|----------|
| C1 | **CSP `connect-src` 过宽**：`'self' http://127.0.0.1:* https://*` 允许 WebView 访问任意 https 主机与本地端口（XSS 时的完整外传/横向链路） | `tauri.conf.json:33` | 收紧为 `connect-src 'self'`（如需再白名单）；补 `object-src 'none'`、`base-uri 'self'`、`frame-src 'none'` |
| C2 | **SSRF**：订阅/Geodata/延迟测试拉取无内网/回环限制且跟随重定向（可指向 `127.0.0.1`、`169.254.169.254`、私网段） | `util/fetch.rs:36-77`、`commands/profiles.rs:260,326`、`geodata/updater.rs:123`、`commands/proxy.rs:28-41` | 解析 URL 后做 IP 段黑名单（回环/私网/链路本地/元数据），限制重定向次数与目标 |
| C3 | **订阅内容可经 mihomo provider `path` 任意文件写入**：`proxy-providers`/`rule-providers` 的 `path` 原样透传（`..` / 绝对路径） | `core/config.rs:205`、`commands/profiles.rs:276` | 对 provider `path` 规范化并强制限定在数据目录内，拒绝 `..`/绝对路径；对不可信订阅做白名单键过滤 |
| C4 | **初始化失败 → 窗口永不显示**：`Promise.all` 中 `configStore.load()` 无容错，任一失败即 `win.show()` 不执行 | `App.vue:24-33`、`stores/config.ts:24-27` | `load()` 加 try/catch + 默认值；`win.show()` 用 try/finally 保证必然执行 |
| C5 | **store 保存失败静默吞掉 / 内存先变**：`save()`/`patch()`/`reset()` 无错误处理，失败后 UI 与后端状态不一致且无提示；`patch` 先改内存后调后端 | `stores/config.ts:28-41`、`SettingsView.vue:97-165` | 先成功调后端再改内存；catch 后 `load()` 回滚 + `ElMessage.error` |
| C6 | **订阅 URL 反射进文件头 → YAML 头注入**：写 `# subscribe-url: {}` 用原始用户字符串（换行可被 reqwest URL 解析剥离但仍原样写盘） | `commands/profiles.rs:276,341` | 写规范化后的 `parsed.as_str()`，转义控制符 |
| C7 | **控制器地址可被改到任意主机 → Bearer secret 外带** | `config/model.rs:177-182`、`core/manager.rs:571-589` | 限制 controller 地址为 `127.0.0.1`/`localhost` |
| C8 | **Geodata 下载无完整性校验、无大小上限** | `geodata/updater.rs:138-190` | 固定 SHA-256 pinning + 20MB 上限 |
| C9 | **系统代理/自启/TUN 命令对前端零门槛**（配合 H1 可劫持全网流量） | `commands/proxy.rs`、`commands/util.rs:40-43` | 开启前校验核心运行 + secret 非默认；敏感操作前端二次确认 |

---

## 4. 低危（发布后处理）

- `devtools` feature 生产启用 + 托盘可随时 `open_devtools()`（`Cargo.toml:20`、`tray/events.rs:195-200`）→ 仅 debug 启用。
- 退出清理按进程名 `taskkill /IM`（`main.rs:332-336`）可能误杀同名进程 → PID 优先。
- 临时文件固定名 + 非原子独占（`config.yaml.tmp`、`runtime-config.yaml.tmp`、`geoip.dat.download`）→ 随机后缀 + `create_new`。
- 导出配置含 secret 与节点密码（`commands/config.rs:47-73`）→ 导出脱敏/文档警示。
- 控制器 REST 客户端无超时（`core/manager.rs:129`）→ 加 10s timeout。
- `CLASH_EDGE_DATA_DIR` 环境变量信任、`list_profiles` 暴露绝对路径 → 信息面收窄。
- 前端双击竞态：ProfilesView 6 个对话框提交、ProxiesView 节点选择、ConnectionsView closeAll、Dashboard restart/reload 无在途守卫（`stores/proxy.ts:33-36`、各 view）→ 加 `submitting`/防抖。
- 测速全量并发无上限（`stores/proxy.ts:59-78`）→ 分块限并发。
- main.ts 常驻监听无 dispose、i18n `unflatten` 原型污染理论面、Vite 6 已过维护期、opener 插件死依赖 → 清理。
- 关闭提示缺失：关闭窗口即隐藏到托盘无提示（`App.vue:102-108`）→ 首次隐藏弹提示。

---

## 5. 正面确认（无需修复）

- `sanitize_profile_name` 路径穿越防护完整（`util/paths.rs:345-391`）。
- 自启动注册表写入、mihomo 启动参数、系统代理注册表值均无命令注入面（`util/autostart.rs`）。
- TLS 校验开启、UA 固定、拉取超时 30s（`util/fetch.rs:21-24`）。
- 单实例保护、退出清理（杀 mihomo + 还原系统代理快照）逻辑正确（`main.rs:67-77, 310-360`）。
- 锁纪律清晰：std 锁（config/tray）不跨 `.await`、tokio 锁（core）可跨；此前死锁已修复并有回归测试。
- 订阅受控键（端口/模式/控制器/TUN/DNS）应用优先，订阅不得覆盖（`core/config.rs:27-38, 186-188`，有测试覆盖）。
- 前端 XSS 面干净：全量文本插值转义、命令名硬编码、无动态组件渲染、webview 默认不可导航外部。
- 密钥启动轮转 + 落盘 + 单元测试（`persistence.rs:39-59` + 2 测试）。

---

## 6. 发布前修复优先级

1. **必须**：H1（密钥轮转收敛到所有落盘路径）+ H2 的 ①②③（关 withGlobalTauri、删 `$HOME/**` 权限、导航锁）。
2. **强烈建议**：C1（CSP 收紧）、C2（SSRF 防护）、C3（provider path 校验）、C4（窗口必现）、C5（store 错误处理）。
3. **建议**：C6/C7/C8/C9。
4. **后续**：第 4 节低危项。

修复后重新执行：`cargo test`（现有 35 项 + 新增）、`cargo check` 0 警告、`npm run build`、`cargo clippy`（标准 stable 工具链）、真实运行冒烟、打包校验 SHA256。