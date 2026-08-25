// src-tauri/src/core/controller.rs
//! 外部控制器 REST 客户端（ControllerClient）
//!
//! CoreManager 拆分（P2）后的独立模块：只依赖 `config`（读取 external-controller /
//! secret）与 `api_client`，不含进程生命周期状态。生命周期仍在 CoreManager；
//! 需要调 REST 的方法（version/get_status 等）仍留在 manager.rs，通过
//! `self.controller` 调用这里的接口。

use std::sync::Arc;
use std::time::Duration;

use parking_lot::RwLock;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use reqwest::Url;
use tracing::{info, warn};

use crate::config::model::Config;
use crate::util::error::{Error, Result};

/// 外部控制器 HTTP 客户端：只做 REST 调用，不持有进程状态。
pub(crate) struct ControllerClient {
    /// 共享配置（与 ConfigManager 同一 Arc，单一数据源）
    config: Arc<RwLock<Config>>,
    /// 外部控制器 HTTP 客户端
    api_client: reqwest::Client,
}

impl ControllerClient {
    pub(crate) fn new(config: Arc<RwLock<Config>>) -> Result<Self> {
        Ok(Self {
            config,
            api_client: reqwest::Client::builder()
                // 低危：REST 客户端统一超时，避免对控制器请求无限阻塞
                .timeout(Duration::from_secs(10))
                .build()?,
        })
    }

    /// 外部控制器基础地址（确保带 http://）
    fn api_base(&self) -> String {
        let addr = self.config.read().proxy.external_controller.clone();
        if addr.starts_with("http://") || addr.starts_with("https://") {
            addr
        } else {
            format!("http://{}", addr)
        }
    }

    /// 构造请求头（Bearer 鉴权）。供仍留在 CoreManager 的流程方法使用。
    pub(crate) fn api_headers(&self) -> Result<HeaderMap> {
        let secret = self.config.read().proxy.secret.clone();
        authorization_headers(&secret)
    }

    /// 暴露底层 HTTP 客户端，供仍留在 CoreManager 的流程（wait_ready /
    /// verify_runtime_applied / reload_config / version）直接发 REST 请求。
    pub(crate) fn api_client(&self) -> &reqwest::Client {
        &self.api_client
    }

    /// 构造控制器 URL；路径段逐段 percent-encode（组名/节点名可含空格、`/`、非 ASCII）。
    pub(crate) fn api_url(&self, path: &[&str], query: Option<&[(&str, &str)]>) -> Result<Url> {
        api_url(&self.api_base(), path, query)
    }

    /// 切换代理模式（PATCH /configs）——只作用于运行中的 mihomo；
    /// 持久化 / 回滚由编排层（apply_proxy_mode）负责。
    pub(crate) async fn set_proxy_mode(&self, mode: String) -> Result<()> {
        let url = self.api_url(&["configs"], None)?;
        let resp = self
            .api_client
            .patch(url)
            .headers(self.api_headers()?)
            .json(&serde_json::json!({ "mode": mode }))
            .send()
            .await?;

        if resp.status().is_success() {
            info!("Proxy mode set to {}", mode);
            Ok(())
        } else {
            warn!("Failed to set proxy mode: {}", resp.status());
            Err(Error::Other(format!(
                "Controller returned {}",
                resp.status()
            )))
        }
    }

    /// 运行中应用 TUN 开关（PATCH /configs {tun:{...}}）。
    /// TUN 变更在 mihomo 中通常需要完整 tun 段；失败时调用方回退整进程重启。
    pub(crate) async fn apply_tun(&self, enable: bool) -> Result<()> {
        let tun = self.config.read().tun.clone();
        let mut tun_value = serde_json::to_value(&tun).unwrap_or_default();
        if let Some(obj) = tun_value.as_object_mut() {
            obj.insert("enable".to_string(), serde_json::Value::Bool(enable));
        }
        let url = self.api_url(&["configs"], None)?;
        let resp = self
            .api_client
            .patch(url)
            .headers(self.api_headers()?)
            .json(&serde_json::json!({ "tun": tun_value }))
            .send()
            .await?;

        if resp.status().is_success() {
            info!(
                "TUN mode {} applied to running core",
                if enable { "enabled" } else { "disabled" }
            );
            Ok(())
        } else {
            warn!("Failed to apply TUN: {}", resp.status());
            Err(Error::Other(format!(
                "Controller returned {}",
                resp.status()
            )))
        }
    }

    /// 获取代理组列表（GET /proxies）。
    /// mihomo 返回的类型名是大写（Selector / URLTest / Fallback / LoadBalance / Relay），
    /// 旧实现只认小写导致永远匹配不到真实代理组。
    pub(crate) async fn get_proxy_groups(&self) -> Result<Vec<serde_json::Value>> {
        let url = self.api_url(&["proxies"], None)?;
        let resp = self
            .api_client
            .get(url)
            .headers(self.api_headers()?)
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(Error::Other(format!(
                "Controller returned {}",
                resp.status()
            )));
        }

        let json: serde_json::Value = resp.json().await?;
        let mut groups = Vec::new();
        if let Some(proxies) = json.get("proxies").and_then(|v| v.as_object()) {
            for (name, value) in proxies {
                let group_type = value
                    .get("type")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string();
                // 真实 mihomo 代理组类型（大小写不敏感，兼容小写旧写法）
                if ["Selector", "URLTest", "Fallback", "LoadBalance", "Relay"]
                    .iter()
                    .any(|t| t.eq_ignore_ascii_case(&group_type))
                {
                    let now = value
                        .get("now")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let all = value
                        .get("all")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    groups.push(serde_json::json!({
                        "name": name,
                        "type": group_type,
                        "now": now,
                        "all": all,
                    }));
                }
            }
        }
        Ok(groups)
    }

    /// 选择代理组中的某个代理（PUT /proxies/{group}，组名 URL 编码）
    pub(crate) async fn select_proxy_group(&self, group: String, proxy: String) -> Result<()> {
        let url = self.api_url(&["proxies", &group], None)?;
        let resp = self
            .api_client
            .put(url)
            .headers(self.api_headers()?)
            .json(&serde_json::json!({ "name": proxy }))
            .send()
            .await?;

        if resp.status().is_success() {
            info!("Selected {} -> {}", group, proxy);
            Ok(())
        } else {
            warn!("Failed to select proxy group: {}", resp.status());
            Err(Error::Other(format!(
                "Controller returned {}",
                resp.status()
            )))
        }
    }

    /// 测试代理组延迟（GET /proxies/{group}/delay）
    pub(crate) async fn test_proxy_latency(
        &self,
        group: String,
        url: Option<String>,
    ) -> Result<Vec<serde_json::Value>> {
        let test_url = url.unwrap_or_else(|| "http://www.gstatic.com/generate_204".to_string());
        // C2 SSRF 防护：该 URL 会作为参数传给 mihomo 由内核去拉取（非本地 client），
        // 同样必须通过禁段校验，防止被当作跳板探测内网。
        crate::util::fetch::validate_url(&test_url).await?;
        let api_url = self.api_url(
            &["proxies", &group, "delay"],
            Some(&[("url", test_url.as_str()), ("timeout", "5000")]),
        )?;

        let req = self
            .api_client
            .get(api_url)
            .headers(self.api_headers()?)
            .send()
            .await?;

        if req.status().is_success() {
            let body: serde_json::Value = req.json().await.unwrap_or_default();
            Ok(vec![serde_json::json!({
                "group": group,
                "delay": body.get("delay"),
            })])
        } else {
            Ok(vec![serde_json::json!({
                "group": group,
                "delay": null,
                "message": format!("HTTP {}", req.status()),
            })])
        }
    }

    /// 获取活动连接（GET /connections）
    /// 返回压缩后的连接列表 JSON（供前端连接面板显示）。
    ///
    /// P2 性能：连接数极大（数千/万级）时不再把全量 JSON 交回 WebView——
    /// 全量链路是 Mihomo JSON → Rust parse → IPC 序列化 → WebView JSON parse →
    /// JS 内存，每一环都随连接数线性膨胀。这里 Rust 侧先压缩并统计 total，
    /// 只把前 `MAX_CONNECTIONS_RETURNED` 条送 WebView，IPC / JSON parse /
    /// JS memory 全部降到常量级。前端用 `total` 展示"共 N 条"，用 `truncated`
    /// 决定是否显示截断提示。
    pub(crate) async fn get_connections(&self) -> Result<serde_json::Value> {
        const MAX_CONNECTIONS_RETURNED: usize = 500;

        let url = self.api_url(&["connections"], None)?;
        let resp = self
            .api_client
            .get(url)
            .headers(self.api_headers()?)
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(Error::Other(format!(
                "Controller returned {}",
                resp.status()
            )));
        }

        let json: serde_json::Value = resp.json().await?;
        let download_total = value_as_u64(json.get("downloadTotal"));
        let upload_total = value_as_u64(json.get("uploadTotal"));

        let mut connections = Vec::new();
        if let Some(arr) = json.get("connections").and_then(|v| v.as_array()) {
            for conn in arr {
                let metadata = conn.get("metadata");
                let host = metadata
                    .and_then(|m| m.get("host"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .or_else(|| {
                        metadata
                            .and_then(|m| m.get("remoteDestination"))
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                    })
                    .or_else(|| {
                        metadata
                            .and_then(|m| m.get("destinationIP"))
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                    })
                    .unwrap_or("")
                    .to_string();

                let network = metadata
                    .and_then(|m| m.get("network"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("tcp")
                    .to_string();

                let conn_type = metadata
                    .and_then(|m| m.get("type"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                let rule = conn
                    .get("rule")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let upload = value_as_u64(conn.get("upload"));
                let download = value_as_u64(conn.get("download"));
                let start = value_as_u64(conn.get("start"));
                let chains = conn
                    .get("chains")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();

                connections.push(serde_json::json!({
                    "id": conn.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    "host": host,
                    "network": network,
                    "type": conn_type,
                    "rule": rule,
                    "upload": upload,
                    "download": download,
                    "start": start,
                    "chains": chains,
                }));
            }
        }

        // P2：只把前 MAX_CONNECTIONS_RETURNED 条送 WebView；total 记真实总数。
        // 前端用 total 显示"共 N 条"，truncated 决定是否渲染截断提示。
        let total = connections.len();
        let truncated = total > MAX_CONNECTIONS_RETURNED;
        connections.truncate(MAX_CONNECTIONS_RETURNED);

        Ok(serde_json::json!({
            "download_total": download_total,
            "upload_total": upload_total,
            "total": total,
            "truncated": truncated,
            "connections": connections,
        }))
    }

    /// 关闭全部活动连接（DELETE /connections）
    pub(crate) async fn close_all_connections(&self) -> Result<()> {
        let url = self.api_url(&["connections"], None)?;
        let resp = self
            .api_client
            .delete(url)
            .headers(self.api_headers()?)
            .send()
            .await?;

        if resp.status().is_success() {
            info!("All connections closed");
            Ok(())
        } else {
            warn!("Failed to close connections: {}", resp.status());
            Err(Error::Other(format!(
                "Controller returned {}",
                resp.status()
            )))
        }
    }
}

/// 构造 mihomo 外部控制器 URL；路径段逐段 percent-encode
/// （组名/节点名可含空格、`/`、非 ASCII，直接拼接会生成非法 URL）。
pub(crate) fn api_url(base: &str, path: &[&str], query: Option<&[(&str, &str)]>) -> Result<Url> {
    let mut url = Url::parse(base)
        .map_err(|_| Error::InvalidArgument(format!("bad external-controller url: {}", base)))?;
    {
        let mut segs = url
            .path_segments_mut()
            .map_err(|_| Error::InvalidArgument("bad controller path".to_string()))?;
        for s in path {
            segs.push(s);
        }
    }
    if let Some(q) = query {
        let mut pairs = url.query_pairs_mut();
        for (k, v) in q {
            pairs.append_pair(k, v);
        }
    }
    Ok(url)
}

/// 构造外部控制器请求头：密钥非空时附带 `Authorization: Bearer <secret>`。
///
/// 密钥含非法 header 字符时显式报错——静默省略 Authorization 会让开启鉴权
/// 的控制器必然返回 401，且报错被误导向「控制器不可达」，难以排查。
/// CoreManager 与 AutoRestartChecker（自愈重启就绪探测）统一走本函数。
pub(crate) fn authorization_headers(secret: &str) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    if !secret.is_empty() {
        let value = HeaderValue::from_str(&format!("Bearer {}", secret)).map_err(|_| {
            Error::Other(
                "控制器密钥包含非法字符（不允许的 HTTP header 字符），请检查配置".to_string(),
            )
        })?;
        headers.insert(AUTHORIZATION, value);
    }
    Ok(headers)
}

/// 从 JSON 值取 u64（兼容整数/浮点，缺失返回 0）
fn value_as_u64(v: Option<&serde_json::Value>) -> u64 {
    v.and_then(|v| v.as_u64())
        .or_else(|| v.and_then(|v| v.as_f64()).map(|f| f as u64))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_api_url_encodes_path_segments() {
        // 组名含空格、斜杠、非 ASCII → 逐段编码
        let url = api_url(
            "http://127.0.0.1:9090",
            &["proxies", "扶梯出行/香港"],
            Some(&[("timeout", "5000")]),
        )
        .unwrap();
        let s = url.to_string();
        assert!(
            s.contains("%2F"),
            "slash in group name must be encoded: {}",
            s
        );
        assert!(s.contains("timeout=5000"), "query preserved: {}", s);
        assert!(
            s.starts_with("http://127.0.0.1:9090/proxies/"),
            "base kept: {}",
            s
        );
    }
}
