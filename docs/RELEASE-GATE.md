# ClashEdge RELEASE-GATE —— 发布前必须逐项实测清单

> 适用版本：自 0.8.8 起的每个候选发布版；1.0 发布前必须全量通过。
> 规则：每一项都必须在真实 Windows 实机（便携包形态）上执行并勾选，任何一项不通过即阻断发布。
> 背景与问题定位见 `docs/AUDIT-0.8.7.md`。
> 当前状态：`main` 为 v1.0.7（commit `6a1ed446e6bbb078a526a4eeee13da0d934f71a3`；本文档更新日期 2026-08-28）。**旧版本（≤ v1.0.6，含 v1.0.5 本轮跳过 Gate 的决定）的自动化/实机结果不作为 v1.0.7+ 的放行证据**，以下历史记录仅作对照。
> **2026-08-27 决定**：经用户确认，v1.0.5 本轮**跳过全部实机代理 Gate**（第四节 A–H、第二节实机项、第六节真实升级项维持未勾选），以当前自动化验收结果收尾发布；实机项留待下轮补测，本文件记录如实保留。**该决定仅适用于 v1.0.5，不适用于后续版本。**

## 当前已知阻断项

- [x] **设置页从文件导入 YAML 的 fs 权限**：已确认修复——前端不再使用 plugin-fs 读文件，统一走后端 Rust command `read_import_file`（前端零 fs 依赖）。2026-08-23 核查。

## 实测记录（2026-08-23，v1.0.0 便携包，脚本化实机测试）
> v1.0.4 的完整实机 Gate 尚未留存全量记录；下表为 v1.0.0 基线实机记录，供对照，不能作为 v1.0.5 放行证据，亦不能作为 v1.0.7 放行证据。

| 测试 | 结果 | 证据 |
| --- | --- | --- |
| 中文+空格路径启动（`测试 目录 GateA/`） | ✅ PASS | App 与 mihomo-win64 均从该路径正常启动 |
| 端口占用：7890 被占时启动核心 | ✅ PASS | mihomo 报 `bind: Only one usage of each socket address`；UI 红条如实呈现错误+可操作提示（点名旧版 Clash.F.Win）；状态保持「已停止」，绝不假运行 |
| 损坏 config.yaml 保护 | ✅ PASS | 非法 YAML 写入后启动：原内容保留为 `config.yaml.corrupt-<ts>.bak`，**不被默认配置覆盖**，应用降级模式可启动 |
| 强杀 mihomo 自动重启 | ✅ PASS | 强杀后 ~1s 自动拉起新进程（新 PID） |
| 连续崩溃熔断（P0-7） | ✅ PASS | 10 分钟窗口内第 3 次强杀后停止自动重启，UI 显示「mihomo exited unexpectedly」Error 态 |

---

## 一、构建与校验

> 下表 2026-08-27 各项为上一轮（v1.0.5 候选）自动化验收记录。**2026-08-28 已对 v1.0.7 逐一复验**，复验结果以本表下方新增记录为准；任何项在当前 v1.0.7 未复验前不构成放行证据。

- [x] `cargo fmt --check`：**v1.0.7 复验通过**（2026-08-28，含本轮全部改动后 clean）
- [x] `cargo clippy --all-targets -- -D warnings`：**v1.0.7 复验通过**（2026-08-28，0 warnings）
- [x] `npm run build`（vue-tsc 类型检查 + Vite 生产构建）：**v1.0.7 复验通过**（2026-08-28，零错误，产物含降级横幅改动）
- [x] `cargo test --all-targets`：**v1.0.7 复验：136/136 通过，1 个需真实 Mihomo/网络的手工 Gate 默认忽略**（2026-08-28；新增：fetch SSRF 白名单/重定向 deadline、update_config_fields 并发、degraded 备份查找等测试；HKCU 仅使用测试专用子键）
- [x] `node node_modules/@tauri-apps/cli/tauri.js build --no-bundle`：**v1.0.7 复验通过**，产出 `target\release\ClashEdge.exe`（2026-08-28）
- [x] `cargo audit`：**v1.0.7 复验：1226 条 RustSec 公告，0 已知漏洞，exit 0**（2026-08-28；17 条 unmaintained/unsound 警告已在 `src-tauri/.cargo/audit.toml` 逐条书面豁免，见上轮记录；本机因 libgit2 不走 git CLI 代理，需注入 `HTTPS_PROXY` 环境变量完成拉取——CI 直连无此问题）
- [x] `npm audit --audit-level=high`：**v1.0.7 复验：0 vulnerabilities**（2026-08-28，官方 registry https://registry.npmjs.org）
- [x] `scripts/windows/build-portable.ps1` 打包成功：**v1.0.7 复验——9/9 invariants + 绝对路径泄露扫描 27 文件 OK**（2026-08-28）
- [x] 产物 zip 与 .sha256 一致，SHA256 校验通过：**v1.0.7** `ClashEdge-portable-win64.zip` = `6e167da704a434a3c27651d9c6fff46d9eb9e345d158a056686eb89c3694d8b5`（2026-08-28）
- [x] Launcher 故障注入 `--test-recovery`：**v1.0.7 复验——48/48 断言 PASS**（2026-08-28；含新增 launcher 自更新断电点 T-l1~T-l8）
- [x] 版本号三处一致：tauri.conf.json / package.json / Cargo.toml = **1.0.7**（2026-08-28 复验一致）
- [x] 产物绝对路径泄露扫描通过（27 个文件，2026-08-28 复验 OK）

## 二、核心生命周期与状态一致性

- [x] 空闲端口启动：内核监听正常，无绑定冲突（2026-08-23）
- [x] **7890 / 9090 / 9053 任一端口被占用时给出明确错误**，UI 呈现 Error，绝不假运行（绑定冲突检测生效）——7890 实测（2026-08-23）
- [ ] 启动/停止/重启各 10 次，状态机无卡死、无僵尸进程残留
  - 2026-08-27 仅完成最终包真实 sidecar 的隔离端口 Gate：启动/停止 10 次、重启 10 次、0 僵尸、0 残留监听；不等同于完整 App 状态机 Gate，故保持未勾选。
- [ ] **多次 restart 后永远只有一个 watcher**（观察事件不重复推送、崩溃后不重复自动重启）
- [ ] Settings 页修改 mixed-port 并保存：runtime-config.yaml 更新、Mihomo 实际监听新端口、系统代理同步新端口（P0-3 验收）
- [ ] Mihomo 拒绝配置时（构造非法值）：UI 显示失败，config.yaml 保持旧值
- [ ] 手工损坏 config.yaml 后启动：原文件保留（备份改名），不被默认配置静默覆盖，用户得到可理解提示（P0-1 验收）
- [ ] reset 配置后运行中内核确实重载为新配置（reload 错误不再被吞）
- [ ] 导入 YAML 配置立即生效（运行中热重载或重启内核成功）
- [ ] 托盘菜单勾选态与实际模式/开关一致（rule/global/direct、系统代理、TUN）

## 三、网络与安全

- [ ] **Task Manager 强杀 ClashEdge 进程，下次启动自动修复系统代理**（Windows 代理不留死端口指向）
- [ ] **强杀 Mihomo 不断网**：系统代理先关闭，按策略自动重启恢复后再恢复代理
- [ ] **500MB 恶意订阅内存不暴涨**：流式下载与大小上限已实现；仍须用真实大响应验证内存曲线和超限错误
- [ ] **localhost 订阅 URL 全拒绝**（SSRF 校验）
- [ ] **private 段（10/172.16/192.168 等）订阅 URL 全拒绝**
- [ ] **DNS rebinding 场景拒绝**（解析结果落私网段的域名订阅源被拒）
- [ ] **redirect 到 127.0.0.1 的订阅 URL 拒绝**（每跳复检或禁止跨源重定向）
- [x] HTTPS LocalProxy fallback 保留 Host/SNI/证书 hostname，同时 SOCKS5 仅接收已校验 IP；GitHub Release redirect、raw GitHub 订阅与 jsDelivr geodata 真实链路通过（2026-08-27）
- [ ] **subscription token 不出现在日志与 UI**（mihomo-stdout/stderr.log、应用日志、错误提示均检查）
- [ ] 控制器密钥非默认占位值，且不出现在 get_config 返回值与导出文件中
- [ ] 测速自定义 URL 同样经过 SSRF 禁段校验
- [ ] allow-lan 开启时有安全提示（若尚未实现，记录为已知风险并评估是否阻断）
- [ ] 用户另开一个自己的 Mihomo 进程时，ClashEdge 退出清理**不能误杀该进程**（按 PID/句柄清理，不按进程名）

## 四、Windows 环境恢复

> 2026-08-27 环境约束：不得改写用户当前真实 Internet Settings 代理键；本节 A-H 代理实机 Gate 未执行，只允许测试专用 HKCU 子键与模拟 ownership 测试，不能作为实机通过证据。

- [ ] 正常退出（托盘 Quit）：系统代理按意图关闭，注册表干净
- [ ] 异常退出（强杀/断电模拟）：下次启动能确定性恢复或清除系统代理，不留死端口
- [ ] mihomo 自动重启健康检查通过后，系统代理按配置意图恢复（P0-5 验收）
- [x] **10 分钟窗口内连续崩溃 3 次：停止自动重启，UI 显示 Error，不无限循环（P0-7 验收）**——实测第 3 次强杀后停止重启、UI 显示「mihomo exited unexpectedly」（2026-08-23）
- [x] mihomo 异常退出后自动拉起新进程——实测强杀后 ~1s 重启（新 PID）（2026-08-23；系统代理恢复路径待系统代理开启场景补测）
- [ ] TUN 开启后异常退出：虚拟网卡/路由不留残留（如适用）
- [ ] 开机自启：注册表 Run 键写入正确，重启系统后随系统启动正常
- [ ] 便携目录整体移动/换盘符后，自启路径在下次启动被自动修复

## 五、便携包

- [x] **中文路径**下解压并正常运行——最终候选 ZIP 在 C 盘中文目录启动并显示 v1.0.5（2026-08-27，`system-proxy: false`）
- [x] **路径含空格**正常运行——同上隔离目录含空格（2026-08-27）
- [ ] **移动盘符变更**（U 盘换机器盘符变化）后 portable.dat 自愈判定仍生效，内核路径解析正确
- [ ] Data 目录随包迁移后配置/Profile/规则完整保留
- [ ] 根 ClashEdge.exe 图标与 App 内应用图标一致
- [x] App/portable.dat、App/clash-edge-core.exe 等打包断言齐备——9/9 invariants OK（2026-08-23）
- [ ] 卸载式删除：直接删目录即可移除（除自启注册表键需说明）

## 六、更新链路（Portable Updater 信任链已实装在 v1.0.3 起；下表为待实机重跑项）

> 现状：v1.0.3 起为完整信任链「编译期公钥 → minisign manifest → manifest SHA256 → ZIP → 暂存 → Launcher 下次启动事务替换」；`download_update()` 无参数、只接受后端刚验签的 manifest，WebView 不能操纵下载。以下仍留待真实升级路径重跑。

- [x] Launcher `--test-recovery`：pending / verified / swapping / committed / 无 journal 共 15 项恢复断言通过（2026-08-27；这是隔离故障注入，不替代真实 v1.0.3 → v1.0.4 升级）

- [ ] **更新中途断电能回滚**：更新事务具备校验点，断电重启后旧版本可用且程序资产一致
- [ ] **更新到新版本后 Data 完整保留**（配置、Profile、规则、日志均不受影响）
- [ ] 更新包完整性校验（哈希/签名）通过才允许替换
- [ ] UI 中任何"检查更新"入口均走真实验签 Flow（无死代码）：实测一次完整升级 + 回滚

## 七、UI 与国际化

- [ ] **1000 节点订阅**导入后 UI 可接受（列表渲染、切换分组无明显卡顿）
- [ ] **100% / 125% / 150% / 175% DPI** 下标题栏、侧栏、对话框无遮挡错位，自绘拖拽区正常
- [ ] light / dark / system 三种主题正确切换，Element Plus 组件与自绘 token 同步，托盘图标深浅任务栏可辨
- [ ] 中英文（zh-CN / en-US）切换完整，无硬编码漏翻关键用户文案
- [ ] 托盘所有菜单项均有真实行为（close_all / move_to_monitor 实装或移除，不得保留 stub）
- [ ] 概览页核心控制三按钮（启动/停止、重启核心、重载配置）状态反馈准确
- [ ] 所有错误提示可操作（含端口占用提示指明具体端口与处理建议）

---


## Android 平台范围说明

- Android 为**实验性/规划中**，**不在本 Gate 范围内**：当前缺少 gradle wrapper（无可执行的 `gradlew` / `gradlew.bat`）、Mihomo AAR/JNI 为占位实现（无真实 VPN 内核集成）、无 release 签名配置，因此无法提供真实 VPN 服务。现状与进入发布范围的前置条件清单见 `apps/android/README.md`。

---

## 通过标准

- 「当前已知阻断项」必须解决；
- 第一至第五、七节全部勾选；
- 第六节在 Phase 3 完成后纳入硬性门槛，1.0 必须全量通过；
- Android 不在本 Gate 范围内（实验性/规划中，见 `apps/android/README.md`）；Windows 便携包为唯一正式发布形态。
