// src-tauri/src/core/health.rs
//! mihomo 就绪 / 端口健康探测与启动日志解析
//!
//! 纯函数辅助：不持有 CoreManager 状态，只处理单个输入。供
//! - CoreManager::start / wait_ready（就绪轮询）
//! - CoreManager::verify_runtime_applied（热重载后真实状态校验）
//! - CoreManager::detect_bind_conflict（启动日志端口冲突检测）

use std::time::Duration;

use crate::util::error::{Error, Result};

/// 端口健康探测（TCP connect）超时
const PORT_PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// TCP 探测 `(host, port)`，超时或拒绝都视为监听失败
pub(crate) async fn probe_tcp<A: tokio::net::ToSocketAddrs>(
    addr: A,
    timeout: Duration,
) -> Result<()> {
    tokio::time::timeout(timeout, tokio::net::TcpStream::connect(addr))
        .await
        .map_err(|_| Error::Other("port probe timed out".to_string()))?
        .map_err(|e| Error::Other(format!("port not listening: {}", e)))?;
    Ok(())
}

/// TCP 探测 "host:port" 字符串地址
pub(crate) async fn probe_str_addr(addr: &str) -> Result<()> {
    match tokio::time::timeout(PORT_PROBE_TIMEOUT, tokio::net::TcpStream::connect(addr)).await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(e)) => Err(Error::Other(format!("{} not listening: {}", addr, e))),
        Err(_) => Err(Error::Other(format!("{} probe timed out", addr))),
    }
}

/// 把 mihomo `dns.listen` 归一化为可探测的 "127.0.0.1:<port>"。
/// 兼容 ":1053" / "0.0.0.0:1053" / "127.0.0.1:1053" 三种写法。
pub(crate) fn normalize_dns_listen(listen: &str) -> String {
    let normalized = listen.replacen("0.0.0.0:", "127.0.0.1:", 1);
    if normalized.starts_with(':') {
        format!("127.0.0.1{}", normalized)
    } else {
        normalized
    }
}

/// 从 mihomo 启动日志文本中提取第一个端口绑定失败（bind）行，返回可读描述。
/// 只匹配 `level=error` 且包含 `bind` 的行（如 mixed-port / DNS 被其他进程占用），
/// 忽略规则拉取等其他 error，避免误报。
pub(crate) fn parse_bind_error(content: &str) -> Option<String> {
    content.lines().find_map(|line| {
        if line.contains("level=error") && line.contains("bind") {
            let detail = line
                .split("msg=")
                .nth(1)
                .unwrap_or_default()
                .trim_matches('"')
                .trim();
            Some(format!(
                "端口绑定失败：{}。请先关闭占用该端口的程序（如旧版 Clash.F.Win 仍在后台运行）后重试。",
                detail
            ))
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_bind_error_detects_port_conflict() {
        // 正常启动日志：无 level=error+bind → None
        let ok = "time=\"...\" level=info msg=\"RESTful API listening at: 127.0.0.1:50715\"\n";
        assert_eq!(parse_bind_error(ok), None);

        // 端口占用：应提取出端口号 + 可操作提示
        let conflict = "time=\"...\" level=error msg=\"Start Mixed(http+socks) server error: listen tcp 127.0.0.1:7890: bind: Only one usage of each socket address (protocol/network address/port) is normally permitted.\"\n";
        let msg = parse_bind_error(conflict).unwrap();
        assert!(msg.contains("7890"), "should name the port: {}", msg);
        assert!(
            msg.contains("关闭占用该端口的程序"),
            "actionable hint: {}",
            msg
        );

        // 规则拉取等其他 error（不含 bind）不误报
        let provider = "time=\"...\" level=error msg=\"[Provider] direct pull error: context deadline exceeded\"\n";
        assert_eq!(parse_bind_error(provider), None);
    }

    // dns.listen 归一化（探测地址必须可连接）
    #[test]
    fn test_normalize_dns_listen() {
        assert_eq!(normalize_dns_listen("127.0.0.1:1053"), "127.0.0.1:1053");
        assert_eq!(normalize_dns_listen("0.0.0.0:1053"), "127.0.0.1:1053");
        assert_eq!(normalize_dns_listen(":1053"), "127.0.0.1:1053");
        assert_eq!(normalize_dns_listen("[::]:1053"), "[::]:1053");
    }
}
