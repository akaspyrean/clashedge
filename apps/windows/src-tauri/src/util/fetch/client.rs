// src-tauri/src/util/fetch/client.rs
//! 受控 HTTP client 与拉取链路：直连优先、代理兜底、手动重定向跟随、
//! 共享总 deadline 的超时语义、resolve pinning（TOCTOU 收敛）。

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use reqwest::header::USER_AGENT;
use reqwest::redirect::Policy;
use reqwest::{Proxy, Url};
use tauri::{AppHandle, Manager};
use tracing::{info, warn};

use super::guards::{redact_url_for_log, validate_url};
use crate::util::error::{Error, Result};

/// 单次拉取超时（与既有订阅拉取行为一致）
const TIMEOUT: Duration = Duration::from_secs(30);

/// 最大重定向跳数（防重定向链被当作跳板无限转发/探测内网）
const MAX_REDIRECTS: usize = 3;

/// 所有 client（直连/代理、首跳/重定向跳）统一的 TCP/TLS 连接超时。
/// 连接阶段不受重定向链总 deadline 的"逐跳剩余时间"精确划分影响，
/// 单独给一个硬上限防止握手挂死。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// 单次读取超时：只约束**一次 read 间隔**（响应头读取、body 相邻 chunk 间隔），
/// 不限制总时长。streaming 模式（无总超时）下载大文件时，只要每次 read 有
/// 进展就不会被掐死；慢速/恶意源的整体卡死仍需调用方以「大小上限 + 整体
/// deadline」兜底。
const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// 每跳最少剩余时间：重定向链共享总 deadline 时，剩余时间低于该值不再发起新跳，
/// 直接判总超时（避免发起注定失败的连接，也覆盖 DNS 校验阶段耗时）。
const MIN_HOP_REMAINING: Duration = Duration::from_millis(100);

/// 拉取请求 User-Agent（部分订阅/下载服务器要求非空 UA）
/// 版本号取自 Cargo 包版本，避免发版后 UA 漂移
fn user_agent() -> String {
    format!("ClashEdge/{}", env!("CARGO_PKG_VERSION"))
}

/// 手动重定向策略：**不自动跟随**，返回重定向信息让调用方做完整异步校验
/// 后再决定是否继续。自动策略的回调是同步的无法做 DNS 校验，故改用手动。
fn no_redirect_policy() -> Policy {
    Policy::none()
}

/// 直连优先、代理兜底地发起 GET 请求。
///
/// 流程：
/// 1. 先用直连 client（`no_proxy`）请求目标 URL；
/// 2. 直连失败（连接失败/超时/非 2xx）则改用应用自身 mihomo 混合端口
///    （`socks5://127.0.0.1:{mixed_port}`）作本地解析的 SOCKS5 代理重试一次；
/// 3. 返回最终 response（调用方负责消费 body / 检查状态码）。
///
/// 返回的 response 状态码不保证是 2xx——调用方需自行判定；
/// 但若直连已拿到 2xx，则不会发起代理重试。
///
/// SSRF + TOCTOU：目标 URL 必须先通过 `validate_url`（含 DNS 白名单检查，
/// 解析失败即拒绝）；校验返回的已验地址用 `resolve()` 钉定到客户端，
/// 避免连接时重新解析。重定向到新主机名时逐跳做完整异步校验再钉定。
pub async fn get_direct_first(app: &AppHandle, url: &str) -> Result<reqwest::Response> {
    get_direct_first_with_timeout(app, url, Some(TIMEOUT)).await
}

/// 大文件下载变体：`total_timeout=None` 时不设总超时（reqwest 的 `timeout()`
/// 覆盖整个请求周期含 body 读取，30s 会掐死几十 MB 的 geoip.dat 下载）。
/// 仅保留连接超时与读取超时（读取超时只约束单次 read 间隔，不限制总时长）；
/// 调用方必须自行以「大小上限 + 整体 deadline」兜底，防止慢速/恶意源把
/// 下载任务无限挂起。
pub async fn get_direct_first_streaming(app: &AppHandle, url: &str) -> Result<reqwest::Response> {
    get_direct_first_with_timeout(app, url, None).await
}

/// 整条请求链（含全部重定向跳）保持同一路由语义。
/// 直连尝试的整个重定向链强制直连；代理兜底的整条链固定走本地代理。
/// 若重定向跳重建 client 时未继承 no_proxy/proxy，会回落到
/// 系统代理/环境变量，导致同一链路前后两跳走不同网络路径。
#[derive(Clone, Copy, PartialEq, Eq)]
enum FetchRoute {
    Direct,
    LocalProxy,
}

/// 把路由语义应用到 client builder（首跳与每个重定向跳统一走这里）
fn apply_route(
    builder: reqwest::ClientBuilder,
    route: FetchRoute,
    proxy_url: &str,
) -> Result<reqwest::ClientBuilder> {
    Ok(match route {
        FetchRoute::Direct => builder.no_proxy(),
        FetchRoute::LocalProxy => builder.proxy(Proxy::all(proxy_url)?),
    })
}

async fn get_direct_first_with_timeout(
    app: &AppHandle,
    url: &str,
    total_timeout: Option<Duration>,
) -> Result<reqwest::Response> {
    // SSRF 防护：目标 URL 必须通过校验（含 DNS 白名单检查，返回已验地址）
    let resolved = validate_url(url).await?;

    // 1. 直连尝试（no_proxy：忽略系统代理/环境代理，强制直连）
    let direct = apply_route(
        build_client_with_resolved(url, &resolved, total_timeout)?,
        FetchRoute::Direct,
        "",
    )?
    .redirect(no_redirect_policy())
    .build()?;
    // 2. 代理兜底：应用自身 mihomo 混合端口
    let proxy_url = local_proxy_url(app);
    let proxied = apply_route(
        build_client_with_resolved(url, &resolved, total_timeout)?,
        FetchRoute::LocalProxy,
        &proxy_url,
    )?
    .redirect(no_redirect_policy())
    .build()?;
    // 总超时一路传递：direct 与 proxied 两条链各自持有同一总预算
    send_direct_then_proxy(&direct, &proxied, url, &proxy_url, total_timeout).await
}

async fn send_direct_then_proxy(
    direct: &reqwest::Client,
    proxied: &reqwest::Client,
    url: &str,
    proxy_url: &str,
    total_timeout: Option<Duration>,
) -> Result<reqwest::Response> {
    match send_and_follow(direct, url, FetchRoute::Direct, "", total_timeout).await {
        Ok(resp) if resp.status().is_success() => return Ok(resp),
        Ok(resp) => {
            warn!(
                "Direct fetch got non-success status {} for {}; retrying via proxy",
                resp.status(),
                redact_url_for_log(url)
            );
        }
        Err(e) => {
            warn!(
                "Direct fetch failed for {}: {}; retrying via proxy",
                redact_url_for_log(url),
                e
            );
        }
    }

    info!(
        "Fetching {} via local proxy {}",
        redact_url_for_log(url),
        proxy_url
    );
    // HTTP CONNECT 代理会收到原始 hostname 并自行解析；而把 HTTPS URL 改成 IP
    // 又会破坏 TLS SNI/证书 hostname 校验。mixed-port 同时支持 SOCKS5：使用
    // `socks5://`（本地解析，不是 socks5h）+ reqwest resolve pinning，SOCKS 请求
    // 只携带已验证 IP，同时 URL/Host/SNI 始终保留原 hostname。
    send_and_follow(
        proxied,
        url,
        FetchRoute::LocalProxy,
        proxy_url,
        total_timeout,
    )
    .await
}

/// 从 URL 提取主机名与已校验地址，构建带 `resolve()` 钉定的 ClientBuilder。
/// 钉定后 reqwest 连接时直接用已校验的 IP，不再重新解析 DNS（关闭 TOCTOU）。
/// 统一施加连接超时与读取超时；`total_timeout` 为该 client 的总超时
/// （重定向跳传剩余时间；streaming 模式传 None，此时仅靠 connect/read
/// 超时与调用方兜底）。
fn build_client_with_resolved(
    url: &str,
    resolved: &[SocketAddr],
    total_timeout: Option<Duration>,
) -> Result<reqwest::ClientBuilder> {
    let parsed =
        Url::parse(url).map_err(|e| Error::InvalidArgument(format!("Invalid URL: {}", e)))?;
    let mut builder = reqwest::Client::builder()
        // 连接阶段硬上限：TCP/TLS 握手挂死不受总超时逐跳划分的精确度影响
        .connect_timeout(CONNECT_TIMEOUT)
        // 读取超时只约束单次 read 间隔（响应头、body chunk 间隔），不限制总时长：
        // streaming 模式依然可慢慢下载大文件，只要每次 read 有进展。
        .read_timeout(READ_TIMEOUT);
    if let Some(t) = total_timeout {
        builder = builder.timeout(t);
    }
    if let Some(host) = parsed.host_str() {
        if !resolved.is_empty() {
            let host_no_brackets = host
                .strip_prefix('[')
                .and_then(|h| h.strip_suffix(']'))
                .unwrap_or(host);
            builder = builder.resolve_to_addrs(host_no_brackets, resolved);
        }
    }
    Ok(builder)
}

/// 发送请求并手动处理重定向：每跳做完整异步 SSRF 校验（含 DNS）后
/// 用钉定连接跟随，避免自动重定向的同步回调无法做 DNS 校验。
/// 重定向跳重建 client 时通过 `apply_route` 保持与首跳相同的
/// 直连/代理路由语义，整条链路网络路径一致。
///
/// 重定向链总 deadline：`total_timeout` 在入口换算为 `Instant` deadline，
/// 整条链（首跳 + 全部重定向跳）共享。每一跳：
/// 1. 剩余时间低于 `MIN_HOP_REMAINING` 直接判总超时（不再发起新跳）；
/// 2. 重定向跳重建 client 时以剩余时间作为该 client 的总超时
///    （`None` 总超时的 streaming 模式除外）；
/// 3. 发送动作用 `tokio::time::timeout(remaining, …)` 双保险包裹——
///    即使 client 总超时在 DNS/连接阶段划分不精确，也不会超过总 deadline。
async fn send_and_follow(
    client: &reqwest::Client,
    start_url: &str,
    route: FetchRoute,
    proxy_url: &str,
    total_timeout: Option<Duration>,
) -> Result<reqwest::Response> {
    let deadline = total_timeout.map(|t| Instant::now() + t);
    let mut current_url = start_url.to_string();
    for _hop in 0..=MAX_REDIRECTS {
        // 每跳剩余时间：deadline 已耗尽（或不足最小阈值）直接判总超时
        let remaining = deadline.map(|d| d.saturating_duration_since(Instant::now()));
        if let Some(r) = remaining {
            if r < MIN_HOP_REMAINING {
                return Err(Error::Other(format!(
                    "request timed out: total deadline nearly exhausted before hop {} \
                     (remaining {:?} < minimum {:?}) for {}",
                    _hop,
                    r,
                    MIN_HOP_REMAINING,
                    redact_url_for_log(&current_url)
                )));
            }
        }
        // 每跳（含首跳）都校验：首跳已由调用方校验并通过 resolve pin 好，
        // 重定向到的新 URL 需要新校验。
        let resolved = if _hop == 0 {
            Vec::new()
        } else {
            validate_url(&current_url).await?
        };
        let req_url = current_url.clone();
        // 对于重定向跳，需要用新 URL 的已验地址重新构建请求；
        // 但 client 是共享的（已钉定首跳主机）。重定向到新主机名时，
        // 我们用独立钉定 client 发送。为关闭 TOCTOU，重定向到新主机时
        // 改用独立钉定 client。
        let req_client = if _hop == 0 {
            // 首跳：client 已由调用方钉定
            client.clone()
        } else if resolved.is_empty() {
            client.clone()
        } else {
            // 重定向跳：原 client 的总超时只覆盖首跳，这里以剩余时间重建，
            // 保证整条链共享同一总 deadline（None 总超时的 streaming 模式除外）
            apply_route(
                build_client_with_resolved(&req_url, &resolved, remaining)?,
                route,
                proxy_url,
            )?
            .redirect(no_redirect_policy())
            .build()?
        };

        let send_fut = req_client
            .get(&req_url)
            .header(USER_AGENT, user_agent())
            .send();
        // 双保险：每跳发送都受剩余时间硬约束（覆盖 DNS/连接阶段与 client
        // 总超时划分不精确的窗口）
        let resp = if let Some(r) = remaining {
            match tokio::time::timeout(r, send_fut).await {
                Ok(inner) => inner?,
                Err(_elapsed) => {
                    return Err(Error::Other(format!(
                        "request timed out: hop {} exceeded remaining {:?} of the shared \
                         total deadline for {}",
                        _hop,
                        r,
                        redact_url_for_log(&req_url)
                    )))
                }
            }
        } else {
            send_fut.await?
        };

        if resp.status().is_success() {
            return Ok(resp);
        }
        if !resp.status().is_redirection() {
            // 非 2xx 且非 3xx：返回给调用方判定（非 2xx 触发代理兜底）
            return Ok(resp);
        }
        // 3xx 重定向：提取 Location，做完整异步校验后跟随
        let location = resp
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| Error::Other(format!("redirect without Location: {}", resp.status())))?;
        // 相对/绝对解析：reqwest 的 Url::join 处理相对 Location
        let next_url = parsed_join(&current_url, location)?;
        warn!(
            "Following redirect {} -> {}",
            redact_url_for_log(&current_url),
            redact_url_for_log(next_url.as_str())
        );
        current_url = next_url.to_string();
    }
    Err(Error::Other(format!(
        "too many redirects (>{}) for {}",
        MAX_REDIRECTS, start_url
    )))
}

/// 将可能相对的 Location 解析为绝对 URL（基于 base）。
fn parsed_join(base: &str, location: &str) -> Result<Url> {
    let base_url = Url::parse(base)
        .map_err(|e| Error::InvalidArgument(format!("invalid base URL: {}: {}", base, e)))?;
    base_url.join(location).map_err(|e| {
        Error::InvalidArgument(format!("invalid redirect Location '{}': {}", location, e))
    })
}

/// 应用自身 mihomo mixed-port 的 SOCKS5 地址。`socks5` 表示客户端解析目标域名；
/// 禁止改成 `socks5h`，后者会让代理端重新 DNS 解析并重开 rebinding 窗口。
fn local_proxy_url(app: &AppHandle) -> String {
    let port = app
        .state::<crate::AppState>()
        .config_manager
        .lock()
        .unwrap()
        .get_config()
        .general
        .mixed_port;
    format!("socks5://127.0.0.1:{}", port)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    /// 相对 Location 解析为绝对 URL
    #[test]
    fn parsed_join_resolves_relative_redirect() {
        let abs = parsed_join("https://example.com/a/b", "/c").unwrap();
        assert_eq!(abs.as_str(), "https://example.com/c");
        let abs = parsed_join("https://example.com/a/b", "c").unwrap();
        assert_eq!(abs.as_str(), "https://example.com/a/c");
        let abs = parsed_join("https://example.com/", "https://other.com/x").unwrap();
        assert_eq!(abs.as_str(), "https://other.com/x");
    }

    // --- SSRF local-proxy fallback 验证 ------------------------------------
    //
    // 约束背景：reqwest 的 `resolve()`（DNS pinning）在 `.proxy(Proxy::all(...))`
    // 模式下不生效——proxy 收到的是原始域名而非已校验 IP。这意味着：
    //   1. validate_url 校验 evil.com → 返回公网 IP（通过）
    //   2. direct 失败 → 触发 local proxy fallback
    //   3. proxy（mihomo）重新解析 evil.com → DNS rebinding 返回 127.0.0.1
    //   4. mihomo 连 127.0.0.1（SSRF 成功）
    //
    // 因此 fallback 采用 SOCKS5 本地解析 + resolve pinning。代理收到已验证 IP，
    // 原 URL hostname 则保留给 HTTP Host / TLS SNI 与证书 hostname 校验。

    #[tokio::test]
    async fn direct_non_success_retries_through_pinned_socks5_fallback() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let direct_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let direct_addr = direct_listener.local_addr().unwrap();
        let direct_task = tokio::spawn(async move {
            let (mut conn, _) = direct_listener.accept().await.unwrap();
            let mut request = [0u8; 1024];
            let _ = conn.read(&mut request).await.unwrap();
            conn.write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n")
                .await
                .unwrap();
        });

        let socks_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let socks_port = socks_listener.local_addr().unwrap().port();
        let socks_task = tokio::spawn(async move {
            let (mut conn, _) = socks_listener.accept().await.unwrap();
            let mut greeting = [0u8; 2];
            conn.read_exact(&mut greeting).await.unwrap();
            let mut methods = vec![0u8; greeting[1] as usize];
            conn.read_exact(&mut methods).await.unwrap();
            conn.write_all(&[5, 0]).await.unwrap();
            let mut header = [0u8; 4];
            conn.read_exact(&mut header).await.unwrap();
            assert_eq!(header[3], 1, "fallback must send a pinned IPv4 target");
            let mut target = [0u8; 6];
            conn.read_exact(&mut target).await.unwrap();
            assert_eq!(&target[..4], &[203, 0, 113, 9]);
            conn.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0])
                .await
                .unwrap();
            let mut request = [0u8; 1024];
            let size = conn.read(&mut request).await.unwrap();
            assert!(String::from_utf8_lossy(&request[..size]).contains("fallback.example.test"));
            conn.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nOK")
                .await
                .unwrap();
        });

        let url = "http://fallback.example.test/resource";
        let direct = apply_route(
            build_client_with_resolved(url, &[direct_addr], Some(Duration::from_secs(5))).unwrap(),
            FetchRoute::Direct,
            "",
        )
        .unwrap()
        .redirect(no_redirect_policy())
        .build()
        .unwrap();
        let proxy_url = format!("socks5://127.0.0.1:{}", socks_port);
        let pinned = SocketAddr::new(Ipv4Addr::new(203, 0, 113, 9).into(), 80);
        let proxied = apply_route(
            build_client_with_resolved(url, &[pinned], Some(Duration::from_secs(5))).unwrap(),
            FetchRoute::LocalProxy,
            &proxy_url,
        )
        .unwrap()
        .redirect(no_redirect_policy())
        .build()
        .unwrap();

        let response = send_direct_then_proxy(
            &direct,
            &proxied,
            url,
            &proxy_url,
            Some(Duration::from_secs(5)),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        direct_task.await.unwrap();
        socks_task.await.unwrap();
    }

    /// integration：SOCKS5 fallback 必须同时满足：
    /// 1) SOCKS connect 目标是 validate_url 已验证的 IP（代理端不做 DNS）；
    /// 2) TLS ClientHello SNI 仍是原 hostname（证书 hostname 校验不被破坏）。
    #[tokio::test]
    async fn socks5_fallback_pins_ip_but_preserves_tls_sni() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_port = listener.local_addr().unwrap().port();
        let proxy_task = tokio::spawn(async move {
            let (mut conn, _) = listener.accept().await.unwrap();
            let mut greeting = [0u8; 2];
            conn.read_exact(&mut greeting).await.unwrap();
            assert_eq!(greeting[0], 5);
            let mut methods = vec![0u8; greeting[1] as usize];
            conn.read_exact(&mut methods).await.unwrap();
            conn.write_all(&[5, 0]).await.unwrap();

            let mut request = [0u8; 4];
            conn.read_exact(&mut request).await.unwrap();
            assert_eq!(&request[..3], &[5, 1, 0]);
            assert_eq!(
                request[3], 1,
                "SOCKS target must be IPv4, not a proxy-resolved domain"
            );
            let mut ip_and_port = [0u8; 6];
            conn.read_exact(&mut ip_and_port).await.unwrap();
            conn.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0])
                .await
                .unwrap();

            let mut client_hello = Vec::new();
            let mut chunk = [0u8; 2048];
            for _ in 0..8 {
                match tokio::time::timeout(Duration::from_millis(250), conn.read(&mut chunk)).await
                {
                    Ok(Ok(0)) | Err(_) => break,
                    Ok(Ok(size)) => {
                        client_hello.extend_from_slice(&chunk[..size]);
                        if client_hello
                            .windows(b"tls-name.example.test".len())
                            .any(|window| window == b"tls-name.example.test")
                        {
                            break;
                        }
                    }
                    Ok(Err(e)) => panic!("failed reading TLS ClientHello: {}", e),
                }
            }
            (ip_and_port, client_hello)
        });

        let url = "https://tls-name.example.test/path";
        let resolved = vec![SocketAddr::new(Ipv4Addr::new(203, 0, 113, 7).into(), 443)];
        let builder =
            build_client_with_resolved(url, &resolved, Some(Duration::from_secs(5))).unwrap();
        let proxy_url = format!("socks5://127.0.0.1:{}", proxy_port);
        let client = apply_route(builder, FetchRoute::LocalProxy, &proxy_url)
            .unwrap()
            .redirect(no_redirect_policy())
            .build()
            .unwrap();

        let send_result = client
            .get(url)
            .header(USER_AGENT, user_agent())
            .send()
            .await;

        let (ip_and_port, client_hello) = tokio::time::timeout(Duration::from_secs(3), proxy_task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&ip_and_port[..4], &[203, 0, 113, 7]);
        assert_eq!(u16::from_be_bytes([ip_and_port[4], ip_and_port[5]]), 443);
        assert!(
            client_hello
                .windows(b"tls-name.example.test".len())
                .any(|window| window == b"tls-name.example.test"),
            "TLS ClientHello must preserve the original hostname as SNI (send={:?}, {} bytes: {:02x?})",
            send_result,
            client_hello.len(),
            &client_hello[..client_hello.len().min(96)]
        );
    }

    /// 重定向链共享总 deadline：任何一跳（含首跳）都不得越过整链总预算。
    ///
    /// 测试设计说明：第二跳重定向目标若用伪造主机名，`validate_url` 的真实
    /// DNS 查询会先行失败（`.test` TLD 无解析），连接根本到不了本地 listener；
    /// 因此用「首跳慢速」验证共享 deadline 的 per-hop 代码路径（与重定向跳
    /// 同一条逻辑：剩余时间检查 → client 超时 → tokio timeout 包裹），并用
    /// remaining 阈值分支与 None（streaming）模式补齐行为断言。
    #[tokio::test]
    async fn redirect_chain_enforces_shared_total_deadline() {
        use tokio::io::AsyncReadExt;

        // 慢速服务器：accept 后读取请求头，然后静默不响应（挂死的一跳）
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            while let Ok((mut conn, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut buf = [0u8; 1024];
                    let _ = conn.read(&mut buf).await;
                    // 保持静默，永不响应
                    tokio::time::sleep(Duration::from_secs(10)).await;
                });
            }
        });

        let url = "http://slow-redirect.example.test/first";
        // client 不设总超时（等价 streaming 模式）：只能靠共享 deadline 兜底
        let client = apply_route(
            build_client_with_resolved(url, &[addr], None).unwrap(),
            FetchRoute::Direct,
            "",
        )
        .unwrap()
        .redirect(no_redirect_policy())
        .build()
        .unwrap();

        // 场景 A：总 deadline = 1s —— 必须在 ~1s（容忍 +500ms）内返回
        // 含超时语义的错误，而不是挂死。
        let start = Instant::now();
        let result = send_and_follow(
            &client,
            url,
            FetchRoute::Direct,
            "",
            Some(Duration::from_secs(1)),
        )
        .await;
        let elapsed = start.elapsed();
        let err = result.expect_err("total deadline must abort the request");
        assert!(
            format!("{}", err).contains("timed out"),
            "error must carry timeout semantics: {}",
            err
        );
        assert!(
            elapsed >= Duration::from_millis(900),
            "must not fail before the deadline: {:?}",
            elapsed
        );
        assert!(
            elapsed <= Duration::from_millis(1500),
            "must respect the 1s deadline (+500ms tolerance): {:?}",
            elapsed
        );

        // 场景 B：None（streaming 模式）不受 deadline 约束——服务器静默、
        // 无 client 总超时、read_timeout 只约束单次 read 间隔，因此请求在
        // 1.5s 内保持挂起（不会"提前失败"也不会"成功返回"）。
        let pending = tokio::time::timeout(
            Duration::from_millis(1500),
            send_and_follow(&client, url, FetchRoute::Direct, "", None),
        )
        .await;
        assert!(
            pending.is_err(),
            "None deadline mode must stay pending beyond 1.5s, got {:?}",
            pending
        );

        // 场景 C：剩余时间不足阈值（100ms）——发送前直接判总超时，立即返回。
        let start = Instant::now();
        let result = send_and_follow(
            &client,
            url,
            FetchRoute::Direct,
            "",
            Some(Duration::from_millis(50)),
        )
        .await;
        let err = result.expect_err("remaining below the minimum threshold must abort");
        assert!(
            format!("{}", err).contains("timed out"),
            "threshold error must carry timeout semantics: {}",
            err
        );
        assert!(
            start.elapsed() <= Duration::from_millis(200),
            "threshold branch must be immediate, took {:?}",
            start.elapsed()
        );

        server.abort();
    }

    /// 手工验证（需真实网络环境）：通过真实 Mihomo mixed-port 跑生产 fallback 路径，覆盖
    /// GitHub Release redirect、常见订阅 CDN 与 geodata CDN。默认忽略；执行时
    /// 设置 CLASHEDGE_TEST_PROXY=socks5://127.0.0.1:<isolated-port>。
    #[tokio::test]
    #[ignore = "requires an isolated real Mihomo and internet access"]
    async fn real_https_fallback_targets_keep_tls_and_redirect_safety() {
        let proxy_url = std::env::var("CLASHEDGE_TEST_PROXY")
            .expect("set CLASHEDGE_TEST_PROXY to an isolated Mihomo SOCKS5 mixed-port");
        for url in [
            "https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/geoip.dat",
            "https://raw.githubusercontent.com/akaspyrean/external/main/rules/direct.yaml",
            "https://cdn.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@release/geosite.dat",
        ] {
            let resolved = validate_url(url).await.unwrap();
            let client = apply_route(
                build_client_with_resolved(url, &resolved, Some(Duration::from_secs(45))).unwrap(),
                FetchRoute::LocalProxy,
                &proxy_url,
            )
            .unwrap()
            .redirect(no_redirect_policy())
            .build()
            .unwrap();
            let response = send_and_follow(
                &client,
                url,
                FetchRoute::LocalProxy,
                &proxy_url,
                Some(Duration::from_secs(45)),
            )
            .await
            .unwrap_or_else(|e| panic!("real proxy fetch failed for {}: {}", url, e));
            assert!(response.status().is_success(), "non-success for {}", url);
        }
    }
}
