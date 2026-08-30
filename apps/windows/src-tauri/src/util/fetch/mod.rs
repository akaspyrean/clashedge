// src-tauri/src/util/fetch/mod.rs
//! 拉取助手：直连优先、直连不通自动切换到应用自身代理兜底
//!
//! 需求：订阅更新、配置更新（geodata）这类"拉取动作"默认走直连；
//! 直连不通时自动改走应用自身 mihomo 混合端口（`127.0.0.1:{mixed_port}`）
//! 的 SOCKS5 本地解析模式重试。软件本身的代理模式不受影响。
//!
//! 判定"直连不通"：直连请求连接失败/超时，或返回非 2xx 状态码
//! （服务器对直连 IP 限流/屏蔽时常见，走代理可绕过）。
//!
//! 安全（SSRF 防护）：所有拉取目标 URL 必须通过 `validate_url` 校验——
//! 采用**严格白名单**语义：scheme 限定 http/https、拒绝 localhost/.local
//! 主机名，且字面 IP 与 DNS 解析结果必须**全部**是"全球可路由公网地址"
//! 才放行。回环/私网/链路本地/CGNAT/TEST-NET/基准测试/多播/保留段等
//! 非全球地址一律拒绝（白名单比黑名单更稳：IANA 新增保留段时默认拒绝）。
//! 另对直连与代理兜底两个 client 统一施加手动重定向处理——跳转前逐跳
//! 做完整异步校验（含 DNS 检查），命中非全球地址即中止，且限制最大跳数。
//! 校验的是**目标 URL**，不是代理地址。
//!
//! 超时语义：整条重定向链共享一个总 deadline（`Instant`），每一跳都用
//! 剩余时间约束（client 总超时 + 每跳 `tokio::time::timeout` 双保险），
//! 并统一施加连接超时与读取超时（streaming 模式仍不设总超时，靠调用方
//! 兜底，见 `get_direct_first_streaming`）。
//!
//! TOCTOU 收敛：`validate_url` 解析 DNS 后返回已校验的地址列表，
//! `get_direct_first` 用 `reqwest::Client::resolve()` 将主机名钉定到
//! 已校验的 IP，避免"校验一次 DNS、连接时重新解析"的窗口。重定向
//! 到新主机名时同样先异步校验再钉定连接。
//!
//! 模块拆分（按单一职责）：
//! - `guards` —— URL 校验 / SSRF 白名单规则 / URL 脱敏；
//! - `client` —— 受控 HTTP client 构建、直连优先 + 代理兜底、重定向链。
//!
//! 公开 API 路径保持 `crate::util::fetch::{validate_url,
//! redact_url_for_log, get_direct_first, get_direct_first_streaming}` 不变。

mod client;
mod guards;

pub use client::{get_direct_first, get_direct_first_streaming};
pub use guards::{redact_url_for_log, validate_url};
