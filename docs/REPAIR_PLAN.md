# ClashEdge Tauri 0.8.5 — 内部修复方案（REPAIR PLAN）

> 生成时间：2026-08-16
> 依据：第一轮全面代码审查（Phase 1）完成后的根因分析与方案设计（Phase 2）。
> 本方案为"内部修改方案"，先于任何代码修改制定；实施顺序遵循 P0 → P1 → P2。

---

## 0. 基线状态（已实测）

- `cargo check`（`tauri-scaffold/src-tauri`）→ **通过**，32 条 warning（几乎全为 dead_code，含 `write_runtime_config`、`extract_mihomo_args`、`enable_uwp_loopback`、`disable_uwp_loopback`、`apply_proxy_config`、`atomic_replace`、`rollback_geodata`、若干 paths 函数、`lib.rs` 等）。
- `npm ci`（`tauri-scaffold`）→ **通过**。
- 前端构建为 `vue-tsc --noEmit && vite build`（无独立 eslint；无 CI 配置）。

---

## 1. 根因汇总（Root Causes）

| # | 根因 | 引发问题 |
|---|------|----------|
| RC1 | **双 Config 状态**：`ConfigManager.config`（权威）与 `CoreManager.config`（启动时快照）分离，且 `CoreManager` 只在 `set_proxy_mode`/`set_tun_mode` 内更新自身快照。 | 重启/重载用陈旧快照覆盖用户修改；模式/TUN 修改后重启丢失；状态漂移。 |
| RC2 | **AppConfig / MihomoConfig 混用且无 catch-all**：`Config` 结构体无 `#[serde(flatten)]` 兜底，订阅 YAML 的未知顶层键（`proxies`、`proxy-providers` 等）在往返后**静默丢失**；`proxy-mode` 写成 mihomo 无法识别的键（应写 `mode:`）。 | 订阅节点丢、模式重启失效、规则/组混乱。 |
| RC3 | **Profile 假激活**：`activate_profile` 只改内存里的 `general.profile` 字符串；`write_config` 又 `shift_remove("profile")`，激活从不落盘；`list_profiles` 里 `active = false` 硬编码 TODO；**从不把 Profile 内容合并进 mihomo 运行时配置**。 | 界面"激活"与真实状态无关；重启后激活丢失；订阅内容从未真正生效。 |
| RC4 | **Profile 名称无净化**：所有 profile 命令 `profiles_dir.join(format!("{}.yaml", name))`。 | 路径穿越（`../`、绝对路径、盘符）；Windows 非法字符/保留名写入失败。 |
| RC5 | **Core 生命周期不完整**：spawn 后立即置 Running（无 REST `/version` 就绪探测）；无子进程异常退出 watcher；stdout/stderr 丢弃；重载=杀进程重启；`close_all` 是 stub；`get_proxy_groups` 只认小写组类型；API 路径无 URL 编码；`move_to_monitor` stub；无单实例；退出清理 `try_lock` 可能失败 + 不还原 AutoConfigURL。 | 启动假成功、崩溃无感知、日志无痕、UI 状态与真实进程不一致。 |
| RC6 | **System Proxy 与 allow_lan 混用**：tray 的"系统代理"勾选改的是 `allow_lan`；`set_system_proxy` 关闭时**删除** ProxyServer/ProxyOverride；快照只 3 字段、无 AutoConfigURL；每次开关调用 `netsh winhttp`。 | 系统代理状态错误、用户原配置被删、WinHTTP 被无关改动。 |
| RC7 | **前端从不调用实时应用命令**：`set_proxy_mode`/`set_tun_mode`/`set_system_proxy` 后端命令从未被前端调用（死代码）；`ProxiesView` 改模式只 `configStore.patch`（仅落盘，不实时）；`ConnectionsView` 轮询无 inFlight 守卫。 | 改设置不生效或不同步；连接轮询并发竞态。 |
| RC8 | **Tray 以 Config 而非 RuntimeState 为投影**：`system_proxy` 勾选来自 `config.general.allow_lan`；代理组数据永远传 `&[]`；菜单 ID 把业务值（组名/节点名）直接拼进 ID 并用 `rsplitn(2,'_')` 解析。 | 托盘勾选错误、代理组子菜单永远空、含下划线的组名/节点名解析错位。 |
| RC9 | **TUN 纯桩**：`tun.rs` 只改内存状态，不碰 mihomo。 | TUN 开关无实际效果。 |
| RC10 | **GeoData 假更新**：`GeoSources::new()` 硬编码 URL（用户配置的 advanced.geo*_url 从不读取）；逐文件非事务式更新；成功后删除备份；`atomic_replace`/`rollback` 死代码。 | 自定义 URL 失效；中途失败无回滚。 |
| RC11 | **Controller 固定密钥**：`default_secret()` = `"clash-edge-secret"`。 | 安全风险；多实例冲突。 |
| RC12 | **发布安全**：CSP `connect-src … https://*` 过宽；capabilities 只 `core:default`；`devtools` feature 无条件启用；无 `bundle.resources`/`externalBin`（sidecar 靠 build-portable.ps1 复制，`tauri build` 产物缺 mihomo）。 | 安全与打包缺口。 |
| RC13 | **i18n 不完整**：`settings.title` 键缺失（SettingsView 用 `$t("settings.title")`，两语言文件都无此键）。 | 设置页标题显示原始键名。 |
| RC14 | **入口分散**：`main.rs`（完整 app）与 `lib.rs`（8 行 stub）各建一个 Builder。 | 移动端/单测入口不一致。 |
| RC15 | **错误处理不统一**：`unwrap`/`expect` 多处（config_manager/std Mutex、tray）；`migration.rs` 解析失败 `unwrap_or_else` 静默。 | 崩溃面与不可诊断性。 |

---

## 2. 目标架构

- **单一 Config 数据源**：`ConfigManager` 持有 `Arc<parking_lot::RwLock<Config>>`；`CoreManager` 共享同一 `Arc`。所有读写走同一把锁。任何修改必须同时完成"内存修改 + 原子落盘"。
- **AppConfig / MihomoConfig 分离**：
  - **AppConfig**（`Data/config.yaml`）：完整应用状态（含 `profile` 激活名、`locale`、`mixin-enabled`、`advanced`、`geodata-mode` 等应用级字段 + 订阅字段 + `#[serde(flatten)]` 未知键兜底）。
  - **MihomoConfig（运行时）**（`Data/runtime-config.yaml`，mihomo 以 `-f` 加载）：由 **激活 Profile 内容 + AppConfig 覆盖**生成，只含 mihomo 顶层合法键（`mode:`、`mixed-port`、`external-controller`、`secret`、`tun`、`dns`、`proxies`、`proxy-groups`、`rules`、`rule-providers` 等），剔除应用级键。
  - 启动/重载 = 重新生成 runtime-config.yaml → REST `PUT /configs`（运行中）或重启（未运行）。
- **状态投影**：界面 / 托盘 / mihomo / Windows 系统代理，均为**同一个 RuntimeState 的不同投影**；任一状态变化统一 `emit` 事件 + 重建托盘菜单。
- **后端统一入口**：`apply_proxy_mode(mode)`、`apply_tun(enable)`、`apply_system_proxy(enable)` 三个编排函数：**校验 → 实时应用（mihomo/注册表）→ 持久化 → 通知 UI/Tray → 回滚**。前端与托盘事件都调用它们，删除死代码。

---

## 3. 实施计划

### P0（任务 #3）—— 阻塞性正确性

1. **P0-A config/model.rs**：`proxy_mode` 改序列化为 `mode`（`#[serde(rename="mode", alias="proxy-mode")]`）；`Config` 增加 `#[serde(flatten)] extra: serde_yaml::Mapping` 兜底；`Default`/测试同步。
2. **P0-B config/persistence.rs**：`ConfigManager` 改持 `Arc<parking_lot::RwLock<Config>>`，暴露 `config_handle()`；`write_config` 不再剔除任何键（AppConfig 完整落盘）；新增 `generate_runtime_config(app:&Config, profile_content:Option<&str>) -> Result<Config>`（或 Value），用于生成 mihomo 运行时配置。init 时若 secret 为默认值则生成随机密钥并落盘。
3. **P0-C core/manager.rs**：`config` 改为共享 `Arc<parking_lot::RwLock<Config>>`；`start()` 生成 runtime-config.yaml 并以 `-f` 启动；spawn 后**轮询 REST `/version` 就绪**再置 Running；`reload_config()` 改 REST `PUT /configs`（运行中）→ 验证 → 失败回滚；捕获 mihomo stdout/stderr 到日志文件；`set_proxy_mode` 改为 `apply_proxy_mode`（校验+实时+持久化由编排层完成）。
4. **P0-D commands/profiles.rs**：新增 `sanitize_profile_name(name)`（拒绝空、`.`/`..`、路径分隔符、绝对路径、盘符、`<>:"|?*`、控制符、Windows 保留名）；全部命令使用之；`create_profile` 把 `Some("")` 视为 None（走默认模板）；`list_profiles` 用 config 里的激活名标记 `active`；`activate_profile` 实现真实激活链（读→校验→持久化激活名→生成运行时→重载→验证→通知 UI/Tray）。
5. **P0-E proxy/system_proxy.rs**：独立 `SystemProxyState{enabled,server,bypass,auto_config_url}`；快照/恢复 4 个注册表值；**关闭时只置 `ProxyEnable=0`，不删 ProxyServer/ProxyOverride**；移除 `netsh winhttp` 调用；删除 `apply_proxy_config`/loopback 死代码（或保留但去调用）。`main.rs` 快照与退出恢复同步改造。
6. **P0-F 统一编排**：新建 `core::runtime`（或 `commands/proxy.rs` 内）`apply_proxy_mode` / `apply_tun` / `apply_system_proxy`，前端 + 托盘事件都改走这里。
7. **P0-G 前端**：
   - `api/config.ts`：`"proxy-mode"` → `"mode"`，补 `proxy-groups`/`rules` 类型。
   - `stores/config.ts`：getter `proxyMode` 读 `mode`。
   - `views/ProxiesView.vue`：`onModeChange` 调 `proxyApi.setProxyMode(mode)`（实时+持久化），不再只 `patch`。
   - `views/SettingsView.vue`：代理模式绑定 `cfg['mode']`；补 `settings.title` i18n 键（两语言文件）。
   - `views/ConnectionsView.vue`：轮询加 inFlight 守卫。
   - `stores/profiles.ts`：`activate()` 激活后重新 `list()`（拿真实 active），不做本地乐观映射。
8. **P0-H Cargo.toml**：加 `rand = "0.8"`（随机密钥）。

### P1（任务 #4）—— 可靠性与一致

1. **P1-A Core 生命周期**：子进程 watcher（`tokio::spawn` 等 `child.wait()`，异常退出→状态 Error + emit）；`CoreStatus` 增 `Crashed`/`Restarting`（可选）；退出清理用可靠 kill（任务杀 + Job Object 可选）；`get_status` 内嵌 version 缓存（避免每次 `-v` 子进程）。
2. **P1-B 代理组类型**：`get_proxy_groups` 识别真实 API 类型 `Selector/URLTest/Fallback/LoadBalance`（小写兼容）；API 路径用 `reqwest::Url::path_segments_mut()` 逐段编码；`select_proxy_group`/`test_proxy_latency`/`get_connections` 同。
3. **P1-C 托盘**：`ProxyGroupInfo` 从真实 `/proxies` 数据构建；`system_proxy` 勾选来自 `SystemProxyState` 投影；菜单 ID 用**不透明 ID + 内部映射表**（`tray/mapping.rs`），`build_tray` 改传真实 proxies；`close_all` 调真实 `close_all_connections`；`move_to_monitor` 用 `monitor_from_point`；`restart`/`quit` 可靠。
4. **P1-D TUN**：`set_tun_mode` 写 `Config.tun.enable` + REST `PATCH /configs {tun:{enable}}` + 持久化；删除 `proxy/tun.rs` 桩或改造成纯投影。
5. **P1-E GeoData**：`update_geodata` 读 `Config.advanced.geo*_url`（回退 `GeoSources::default()`）；**事务式**：先下载全部→校验（魔数/大小阈值）→统一备份→统一替换→commit；失败整体回滚；删除 `atomic_replace` 死代码或复用。
6. **P1-F 单实例**：加 `tauri-plugin-single-instance`。
7. **P1-G 打包**：`tauri.conf.json` 加 `bundle.resources`（sidecar/规则文件/geodata 默认文件）；新建 `scripts/prepare-sidecars.ps1`（下载/校验 SHA256/放置）；`tools/build-portable.ps1` 复用。CSP 收紧为 `connect-src 'self' http://127.0.0.1:*`；capabilities 补齐前端用到的权限；`devtools` feature 仅 debug。
8. **P1-H 订阅导入安全**：仅 http/https；重定向跟随；超时 30s；大小上限（如 5MB）；失败不落盘。

### P2（任务 #5）—— 打磨与测试

1. **P2-A 错误处理**：清除 `unwrap`/`expect`（config_manager/tray/路径）；`Error` 增加上下文；`migration.rs` 不再静默吞错。
2. **P2-B i18n 完整性**：全量核对 `zh-CN/en-US` 键与前端/托盘用键；补 `settings.title` 等缺失键。
3. **P2-C 单元测试**：Config roundtrip（含未知键兜底、mode 别名）、Profile sanitize、URL 编码、RuntimeState 投影、SystemProxy 快照/恢复、运行时配置生成、激活链。
4. **P2-D 静态检查**：`npm run build`、`cargo fmt --check`、`cargo clippy -- -D warnings`（或先清 warnings 再开 -D）、`cargo test`。
5. **P2-E 旧 0.8.5 配置迁移**：版本化、可重复、先备份再迁移；兼容 `proxy-mode` 旧键 → `mode`。
6. **P2-F 入口统一**：`lib.rs` 与 `main.rs` 共用同一 Builder（`run()` 移入 lib 或让 lib 委托 main）。

### 阶段六（任务 #6）—— 构建与回归
`cargo tauri build`（真实产物）、smoke、异常场景、性能检查、回归清单逐项验收。

### 阶段七（任务 #7）—— 最终报告
修复摘要 / Root Cause / P0-P2 修复清单 / 架构变化 / 修改文件清单 / 删除内容 / Build 结果 / 测试结果 / 已知剩余问题 / 发布判断。

---

## 4. 禁止项复核（承诺遵守）

- 不 catch-and-ignore 错误、不强制 Running、不强制 active=true、不在失败时返回 Ok、不滥用 `any`/`@ts-ignore`、不全局 allow dead_code、不关 Clippy、不删 CSP、不要求管理员运行、不要求手工改注册表、不要求删配置、不删除功能来规避 Bug。

## 5. 边界分工（多智能体）

- 修改存在冲突风险的集中文件（model.rs / persistence.rs / manager.rs / profiles.rs / 前端 config 键）：由 **Lead** 顺序实施。
- 独立文件（system_proxy.rs、geodata/updater.rs、i18n yaml、ConnectionsView.vue）可由 **Implementer** 并行，**Reviewer** 独立核验，Lead 汇总后必须自查。
- 避免两个 Agent 同改一文件。
