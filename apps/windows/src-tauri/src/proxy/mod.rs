// src-tauri/src/proxy/mod.rs
//! Proxy module - system proxy
//!
//! 注：UWP 回环豁免（loopback）与 TUN 状态机桩（tun）已是死代码：
//! - 真实 TUN 走 mihomo 原生 `tun.enable`（CoreManager::apply_tun），
//!   不存在任何 sidecar 状态机；
//! - 回环豁免从未被任何调用方接线，且 TUN/系统代理用不到。
//!
//! 两者已移除，避免误导后续维护者。

pub mod journal;
pub mod system_proxy;
