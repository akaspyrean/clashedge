# ClashEdge 0.8.7 发布级审查报告（Phase 0 基线审计）

> 审计基线：commit `9c6b203` / v0.8.7（tauri.conf.json version = 0.8.7）
> 审计日期：2026-08-22
> 性质：只读审查，未修改任何代码。本文件是 Phase 0 冻结基线的核心产出，配套实测清单见 `docs/RELEASE-GATE.md`。

---

## 1. 总体结论

| 维度 | 结论 | 说明 |
| --- | --- | --- |
| 基础安全 | 较好 | 前端不直连外部网络、CSP 收紧、控制器密钥随机轮换且不回传 WebView、SSRF 禁段校验覆盖订阅与测速 URL |
| 核心功能 | 较完整 | 生命周期 / Profile / 托盘 / 系统代理 / TUN / geodata / 多语言均已可用，端口绑定冲突不再假成功 |
| 状态一致性 | **主要短板** | Settings 页保存不重载核心；ConfigManager 先改内存后落盘；reset/import 吞 reload 错误 |
| 失败恢复 | **主要短板** | mihomo 崩溃自愈后不恢复系统代理；watcher 无代际控制可多实例竞态；崩溃循环计数可被清零 |
| Portable 形态 | 较成熟 | App/Data 分离、portable.dat 自愈判定、自启路径自动修复、打包前后置校验齐备 |
| 更新机制 | **未完成** | tauri-plugin-updater 已注册、tauri.conf.json 有 endpoints 但 pubkey 为空，Rust/前端均无更新检查逻辑——半成品 |
| UI | 已过能用阶段 | 设计系统 token 化、深浅主题、托盘着色完成；仍有 stub 菜单（close_all / move_to_monitor）暴露给用户 |
| 自动测试 | 偏弱 | 仅 Rust 单元测试（纯函数为主），无集成测试、无前端测试、无端到端回归 |
| CI / 供应链 | 基础级 | ci.yml + release workflow（幂等 draft、并发锁）已有；无依赖审计、无 artifact 校验门禁、无签名 |
| 文档 | 已漂移 | HANDOVER.md 停留在 0.8.5，启动器表述与实际构建流程不符 |

一句话结论：**产品形态已经立住，但「状态一致性与失败恢复」两条主线存在成体系的 P0 缺陷，更新机制是半成品。在修复 P0 并通过 RELEASE-GATE 之前，不应将任何后续版本称为"稳定版"。**

---

## 2. P0 问题清单

### P0-1 配置迁移可能静默丢数据

- 位置：`apps/windows/src-tauri/src/config/persistence.rs` `read_config`（约 163-196 行）；`apps/windows/src-tauri/src/config/migration.rs` `migrate_mixed_format` / `migrate_from_yaml_string` / `is_new_format`
- 现象：`read_config` 在 YAML 解析失败时调用 `migration::migrate`；一旦迁移失败，直接 `warn!` 后返回 `Ok(Config::default())`（persistence.rs:189-193）。返回值随后会被 `init` → `set_config` 落盘，**用户旧配置被默认配置静默覆盖，无任何用户可见错误**。
- 加重因素：
  - `migrate_mixed_format`（migration.rs:293-301）是空壳：不读内容、不改文件，却报告"迁移成功"，导致 `read_config` 重读原样内容再次解析失败；
  - `migrate_from_yaml_string`（migration.rs:320-327）同为空壳；
  - `is_new_format` 恒返回 `true`（migration.rs:315-317），混合格式的分支实际不可达。
- 影响：损坏的 config.yaml（BOM 已处理，但截断/非法语法仍会触发）会让用户丢失全部个性化配置。
- 修复方向：迁移失败时保留原文件并进入显式恢复流程（备份 + 提示），绝不以默认配置覆盖；补齐或删除空壳迁移路径。

### P0-2 ConfigManager 先改内存再落盘，磁盘写失败导致内存/磁盘不一致

- 位置：`apps/windows/src-tauri/src/config/persistence.rs` `set_config`（85-90 行）
- 现象：`set_config` 先 `*self.config.write() = config` 再 `save()`。若 `atomic_write` 失败（磁盘满、权限、占用），函数返回 Err 但内存已是新值。此后所有读取（含 CoreManager 共享的同一 Arc）都基于新值，而磁盘仍是旧值；下次启动状态回跳。
- 影响：违反「用户看到成功意味着最终状态真的成功」与「任意中间步骤失败都恢复到操作前状态」。
- 修复方向：disk-first（先写盘成功后再提交内存），或落盘失败时回滚内存并向上传播错误。

### P0-3 Settings 页 update_config 只刷新托盘不重载 Mihomo（"假保存"）

- 位置：`apps/windows/src-tauri/src/commands/config.rs` `update_config`（33-43 行）
- 现象：Settings 页整包保存走 `update_config` → 落盘 → `refresh_tray`，**不重建 runtime-config.yaml、不 reload 运行中的核心**。修改 mixed-port / DNS / TUN 等字段后 UI 显示已保存，Mihomo 实际仍在旧配置上运行；仅 reset/import 路径有 `reload_running_core`。
- 影响：直接违反硬原则 1（UI 显示的必须是真实状态）与硬原则 2（成功即真成功）。这是当前最容易被普通用户踩中的 P0。
- 修复方向：update_config 事务化——持久化成功后统一走 `regen_runtime_config` + `reload_running_core`，失败回滚并报错。

### P0-4 reset/import 中 reload_running_core 吞掉 reload 错误仍返回成功

- 位置：`apps/windows/src-tauri/src/commands/config.rs` `reload_running_core`（117-125 行）
- 现象：`core.reload_config().await` 出错时仅 `warn!` 日志，函数正常返回，命令整体返回 Ok。用户看到"重置/导入成功"，但运行中核心可能仍是旧配置甚至处于异常态。
- 修复方向：把 reload 结果纳入命令返回值；失败时明确告知用户"已写入但生效失败"，并提供重试/重启内核的路径。

### P0-5 mihomo 崩溃自愈后不恢复 Windows 系统代理

- 位置：`apps/windows/src-tauri/src/core/manager.rs` watcher（约 408-617 行）
- 现象：watcher 检测到异常退出时，若系统代理开着会立即关闭它（防断网，正确）；但自动重启成功、`wait_ready_and_check_port` 通过后（manager.rs:576-586），**只置 Running + 清零计数，从不按配置意图重新打开系统代理**。用户的配置里 system_proxy=true，实际 Windows 代理保持关闭，直到手动干预。
- 影响：崩溃一次 = 系统代理永久失效（对用户表现为"代理不好使了"），违反硬原则 5 的对称性要求。
- 修复方向：自动重启健康检查通过后，按共享配置的 system_proxy 意图恢复注册表设置。

### P0-6 每次 start() spawn 新 watcher，无 generation/cancel，多 watcher 竞态

- 位置：`apps/windows/src-tauri/src/core/manager.rs` `start()` 尾部 `spawn_watcher()`（约 319 行）
- 现象：每次 `start()` 成功都会再 spawn 一个 watcher 任务，旧 watcher 只在"用户主动停止"时 break。restart 场景下 stop→start 会累积新 watcher；旧 watcher 与新 watcher 同时轮询同一个 child Arc，可能出现重复检测退出、重复触发 Error 事件、重复自动重启。
- 影响：多次 restart 后行为不可预测，是若干"偶发双事件/双重启"类问题的结构性根因。
- 修复方向：引入 generation 计数或 CancellationToken，start 时作废旧 watcher；长期应收敛为单一 CoreSupervisor 任务（见 Phase 1 PR-4）。

### P0-7 自动重启成功即清零计数，短周期崩溃循环永不停止

- 位置：`apps/windows/src-tauri/src/core/manager.rs` watcher 重启成功分支（约 578-579 行）
- 现象：`auto_restart_count` 在每次自动重启成功达到 Running 即清零。若 mihomo 以短于退避周期的规律崩溃（例如每 10 秒崩一次），计数永远是 0→1→0→1，`MAX_AUTO_RESTARTS=3` 永远达不到，无限重启循环不会停止。
- 修复方向：改为时间窗熔断（如 10 分钟窗口内崩溃满 N 次即停止重启并置 Error），窗口不重置已计入的崩溃次数。

---

## 3. P1 问题清单（简述）

| 编号 | 问题 | 要点 |
| --- | --- | --- |
| P1-a | Profile 操作无事务性 | 删除/导入/激活多步文件操作中途失败会留下半成品目录与不一致的 profile 字段 |
| P1-b | 10MB 订阅限制不能限制内存 | `commands/profiles.rs` 用 `resp.text().await` 先全量读入内存再校验长度，500MB 恶意响应照样打爆内存；应流式下载边写边计数 |
| P1-c | 订阅 URL token 可能落日志 | `# subscribe-url:` 注释头与部分日志/错误信息可能带出带 token 的完整 URL |
| P1-d | redirect 链路由语义丢失 | reqwest 默认跟随 redirect 后，原始 URL 的 SSRF 校验结果不能代表最终目标（需限制重定向并对每跳复检） |
| P1-e | allow-lan 无安全提示与高级控制 | 打开 allow-lan 即向局域网暴露代理端口，无确认弹窗、无绑定接口选择、无访问控制 |
| P1-f | Junction 目标未验证 | 便携模式相关路径若经 junction/symlink，未验证目标合法性 |
| P1-g | 退出时按进程名杀 mihomo 可能误杀 | 兜底清理若按进程名匹配，用户自己另开的 mihomo 也会被杀；应以记录的 PID/子进程句柄为准 |
| P1-h | 系统代理无 Recovery Journal | ClashEdge 崩溃/被强杀后无人知道"上次是否开了系统代理"，只能靠下次启动启发式修复；需要持久化的代理意图日志支撑确定性恢复 |
| P1-i | 托盘 MenuId 编码真实名称有歧义 | 用分隔符拼接 MenuId 承载组名/节点名，名称本身含分隔符时会错解 |
| P1-j | 托盘暴露未实现 stub 菜单 | `tray/builder.rs` 构建 close_all / move_to_monitor 菜单项，`tray/events.rs` 对应分支仅打日志（stub），用户点击无效果 |
| P1-k | log-level 假设置 | 设置里的 log-level 不影响运行中内核的日志级别，也无热更路径 |
| P1-l | TUN 双入口 | 设置页开关与概览页/托盘路径并存，语义与时序不完全一致 |
| P1-m | Settings 文件导入 fs 权限待实测 | 前端 `open()` + `readTextFile()`，capability 仅有 `dialog:default` 无 fs 读权限，预计运行时报权限拒绝（见 RELEASE-GATE 阻断项） |

---

## 4. 工程目标六条硬原则

所有后续开发必须以下列为验收前提：

1. **UI 显示的必须是真实状态** —— 界面上的每个状态位都要能追溯到 Mihomo 或 Windows 的真实反馈，禁止"乐观置位"。
2. **用户看到成功意味着最终状态真的成功** —— 返回 Ok 之前，持久化、运行时下发、外部副作用必须全部确认完成。
3. **任意中间步骤失败都恢复到操作前状态** —— 多步操作必须具备回滚路径，禁止留下半成品。
4. **任何网络输入都不能突破本地安全边界** —— 订阅 URL、测速 URL、导入 YAML、节点配置均视为不可信输入。
5. **ClashEdge 崩溃不能把 Windows 网络环境留坏** —— 系统代理的开启/关闭必须有确定性的恢复机制，不依赖应用存活。
6. **更新 ClashEdge 不能碰用户 Data** —— 更新/升级只替换程序资产，Data/ 目录完整性是不可侵犯边界。

---

## 5. 分阶段路线图

```
Phase 0  0.8.7   冻结基线（本文档 + RELEASE-GATE.md，代码不动）
Phase 1  0.8.8   状态一致性（6 个 PR）
Phase 2  0.8.9   安全与异常恢复
Phase 3  0.8.10  发布与更新链（Portable Updater 重做，移除半成品 Tauri updater）
Phase 4  0.9.0   UI 与产品完成度
Phase 5  0.9.x   代码治理
Phase 6  1.0     Release Gate（全量通过 docs/RELEASE-GATE.md）
```

### Phase 1（0.8.8 状态一致性）PR 列表

| PR | 内容 | 对应问题 |
| --- | --- | --- |
| PR-1 安全迁移 | 迁移失败保留原文件 + 显式恢复流程；删除空壳迁移路径 | P0-1 |
| PR-2 disk-first 提交 | set_config 先落盘成功再提交内存，失败回滚 | P0-2 |
| PR-3 Settings 事务化 | update_config 统一走 regen_runtime_config + reload，失败回滚并如实报错 | P0-3, P0-4 |
| PR-4 CoreSupervisor + 熔断 | 单一监督任务替代散装 watcher（generation/cancel），时间窗熔断 | P0-6, P0-7 |
| PR-5 崩溃后代理恢复 | 自动重启健康后按配置意图恢复系统代理 | P0-5 |
| PR-6 Profile 事务 | Profile 删除/导入/激活操作原子化 | P1-a |

### Phase 2-6 要点

- **Phase 2（0.8.9）**：订阅流式下载与大小限制（P1-b）、URL token 防泄露（P1-c）、redirect 每跳复检（P1-d）、allow-lan 安全提示（P1-e）、Recovery Journal（P1-h）、退出清理按 PID（P1-g）。
- **Phase 3（0.8.10）**：Portable 自更新链路重做（清单校验 + 断电回滚），移除 tauri.conf.json 中 pubkey 为空的 updater 配置与 main.rs 的插件注册，避免"看起来支持更新"。
- **Phase 4（0.9.0）**：托盘 stub 菜单实装或移除（P1-j）、MenuId 语义化（P1-i）、log-level 真设置（P1-k）、TUN 入口归一（P1-l）、大节点量渲染优化。
- **Phase 5（0.9.x）**：模块边界整理、错误类型统一、测试补强（集成测试 + 关键路径自动化）、CI 增加依赖审计与产物校验。
- **Phase 6（1.0）**：docs/RELEASE-GATE.md 全部项目实测通过后方可发布。

---

## 6. 0.8.8 验收标准

0.8.8 发布前，以下场景必须全部实测通过（与 RELEASE-GATE 对应项联动）：

1. **端口修改真实生效**：Settings 页修改 mixed-port 保存后，runtime-config.yaml 更新、Mihomo 实际监听新端口（netstat/控制器确认）、Windows 系统代理同步指向新端口。
2. **拒绝配置时如实失败**：构造 Mihomo 会拒绝的配置保存时，UI 显示失败，config.yaml 保持旧值，运行中核心不受影响。
3. **崩溃后代理先关后恢复**：kill mihomo 进程后，Windows 系统代理先被关闭（不死端口），自动重启健康检查通过后按配置意图自动恢复。
4. **熔断有效**：10 分钟内连续崩溃 3 次，停止自动重启，UI 呈现明确的 Error 状态而非无限循环。
5. **损坏配置不丢数据**：手工损坏 config.yaml 后启动，原文件被保留（改名备份等），不得被默认配置静默覆盖，用户得到可理解的提示。

---

## 附：关键代码索引

| 主题 | 位置 |
| --- | --- |
| 配置读写 / ConfigManager | `src-tauri/src/config/persistence.rs` |
| 迁移逻辑（空壳） | `src-tauri/src/config/migration.rs` |
| 配置命令（假保存/吞错） | `src-tauri/src/commands/config.rs` |
| CoreManager / watcher | `src-tauri/src/core/manager.rs` |
| 统一编排层 | `src-tauri/src/core/runtime.rs` |
| 托盘菜单与 stub | `src-tauri/src/tray/builder.rs`, `src-tauri/src/tray/events.rs` |
| 订阅拉取（内存限制失效） | `src-tauri/src/commands/profiles.rs` |
| updater 半成品 | `src-tauri/tauri.conf.json` plugins.updater（pubkey 空）、`src-tauri/src/main.rs` 插件注册 |
| capability（缺 fs 权限） | `src-tauri/capabilities/default.json` |
