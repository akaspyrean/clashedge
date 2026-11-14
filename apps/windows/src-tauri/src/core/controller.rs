// src-tauri/src/core/controller.rs
//! 外部控制器 REST 客户端（ControllerClient）
//!
//! 常规 REST 调用经 Mihomo Windows named pipe 传输，pipe 名由共享配置中的
//! per-install secret 派生；`/logs` 长连接仍由 logs 模块通过 loopback HTTP 获取。
//! CoreManager 负责进程生命周期；manager/lifecycle/supervisor 通过此模块调用 API。

use std::sync::Arc;

use parking_lot::RwLock;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use reqwest::Url;
use tracing::{info, warn};

use crate::config::model::Config;
use crate::core::pipe_http::PipeHttpClient;
use crate::util::error::{Error, Result};

pub(crate) fn controller_pipe_name(secret: &str) -> String {
    use sha2::{Digest, Sha256};

    use std::fmt::Write as _;

    let digest = Sha256::digest(secret.as_bytes());
    let mut suffix = String::with_capacity(32);
    for byte in &digest[..16] {
        let _ = write!(suffix, "{byte:02x}");
    }
    format!(r"\\.\pipe\ClashEdge-Mihomo-Controller-{suffix}")
}

/// 会话覆盖优先、其次配置值的控制器地址。
pub(crate) fn effective_controller_addr(
    config: &RwLock<Config>,
    controller_override: &RwLock<Option<String>>,
) -> String {
    controller_override
        .read()
        .clone()
        .unwrap_or_else(|| config.read().proxy.external_controller.clone())
}

fn strip_multiplier_suffix(name: &str) -> &str {
    let trimmed = name.trim();
    for marker in [" ×", " x", " *"] {
        if let Some((base, suffix)) = trimmed.rsplit_once(marker) {
            let value = suffix.trim().trim_start_matches(['x', 'X', '×', '*']);
            if value.parse::<f64>().is_ok() {
                return base.trim_end();
            }
        }
    }
    trimmed
}

fn edit_distance(left: &str, right: &str) -> usize {
    let right: Vec<char> = right.chars().collect();
    let mut row: Vec<usize> = (0..=right.len()).collect();
    for (i, a) in left.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, b) in right.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = (row[j + 1] + 1)
                .min(row[j] + 1)
                .min(diagonal + if a == *b { 0 } else { 1 });
            diagonal = above;
        }
    }
    row[right.len()]
}

fn tun_patch_payload<T: serde::Serialize>(tun: &T, enable: bool) -> Result<serde_json::Value> {
    let mut tun_value = serde_json::to_value(tun)?;
    if let Some(object) = tun_value.as_object_mut() {
        object.insert("enable".to_string(), serde_json::Value::Bool(enable));
    }
    Ok(serde_json::json!({ "tun": tun_value }))
}

fn parse_tun_enable(live: &serde_json::Value) -> bool {
    live.get("tun")
        .and_then(|tun| tun.get("enable"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

fn fuzzy_proxy_match<'a>(stale: &str, candidates: &[&'a str]) -> Option<&'a str> {
    let wanted = strip_multiplier_suffix(stale).to_lowercase();
    let mut ranked: Vec<(usize, &str)> = candidates
        .iter()
        .map(|candidate| {
            let normalized = strip_multiplier_suffix(candidate).to_lowercase();
            (edit_distance(&wanted, &normalized), *candidate)
        })
        .collect();
    ranked.sort_by_key(|(distance, _)| *distance);
    let (best, name) = ranked.first().copied()?;
    if ranked.get(1).is_some_and(|(second, _)| *second == best) {
        return None;
    }
    let name_len = wanted.chars().count();
    // Short labels carry little identifying information; only allow an exact
    // normalized match below six characters.
    let threshold = if name_len < 6 {
        0
    } else {
        (name_len / 5).min(3)
    };
    (best <= threshold).then_some(name)
}

/// 外部控制器 HTTP 客户端：只做 REST 调用，不持有进程状态。
pub(crate) struct ControllerClient {
    /// 共享配置（与 ConfigManager 同一 Arc，单一数据源）
    config: Arc<RwLock<Config>>,
    /// 本次会话的控制器地址覆盖（配置端口被占用时改选的空闲端口）。
    /// 只存在于运行期：绝不写回共享配置（共享配置会被任意设置保存落盘）。
    controller_override: Arc<RwLock<Option<String>>>,
    /// Mihomo named-pipe controller HTTP client.
    api_client: PipeHttpClient,
}

impl ControllerClient {
    pub(crate) fn new(config: Arc<RwLock<Config>>) -> Result<Self> {
        let pipe_config = config.clone();
        Ok(Self {
            config,
            controller_override: Arc::new(RwLock::new(None)),
            api_client: PipeHttpClient::new_dynamic(move || {
                controller_pipe_name(&pipe_config.read().proxy.secret)
            }),
        })
    }

    /// 覆盖句柄（CoreManager / watcher 共享同一份）
    pub(crate) fn override_handle(&self) -> Arc<RwLock<Option<String>>> {
        self.controller_override.clone()
    }

    /// 当前生效的控制器地址：会话覆盖优先，其次配置值。
    pub(crate) fn effective_addr(&self) -> String {
        effective_controller_addr(&self.config, &self.controller_override)
    }

    /// 外部控制器基础地址（确保带 http://）
    fn api_base(&self) -> String {
        let addr = self.effective_addr();
        if addr.starts_with("http://") || addr.starts_with("https://") {
            addr
        } else {
            format!("http://{}", addr)
        }
    }

    /// Named-pipe requests are authorized by the Windows pipe ACL; Mihomo does
    /// not validate the HTTP secret on this transport, so do not send a misleading
    /// Bearer header.
    pub(crate) fn api_headers(&self) -> Result<HeaderMap> {
        Ok(HeaderMap::new())
    }

    /// Named-pipe HTTP client shared by lifecycle, manager and supervisor probes.
    pub(crate) fn api_client(&self) -> &PipeHttpClient {
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
        let patch = tun_patch_payload(&tun, enable)?;
        let url = self.api_url(&["configs"], None)?;
        let resp = self
            .api_client
            .patch(url)
            .headers(self.api_headers()?)
            .json(&patch)
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

    /// 读取运行中核心的实际 TUN 状态（GET /configs → tun.enable）。
    ///
    /// 「确认实际结果」的核心：PATCH /configs 返回 200 不代表新配置真正生效，
    /// mihomo 可能静默跳过非法字段或 listen 失败。必须回读当前运行配置核对
    /// `tun.enable`，与编排层 `apply_tun` 的所需目标值比对。
    ///
    /// 厂商约定：mihomo 运行时配置始终携带 `tun` 段（我们 build_runtime_config
    /// 无条件写 `tun`），`tun.enable` 缺省/异常时视为 false（= 内核未启用 TUN）。
    pub(crate) async fn get_tun_enable(&self) -> Result<bool> {
        let url = self.api_url(&["configs"], None)?;
        let resp = self
            .api_client
            .get(url)
            .headers(self.api_headers()?)
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(Error::Other(format!(
                "read-back TUN state failed: Controller returned {}",
                resp.status()
            )));
        }
        let live: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::Other(format!("read-back TUN state decode failed: {}", e)))?;
        Ok(parse_tun_enable(&live))
    }

    /// 获取代理组列表（GET /proxies）。
    /// mihomo 返回的类型名是大写（Selector / URLTest / Fallback / LoadBalance / Relay），
    /// 类型匹配必须按实际返回的大小写处理，不得假设全小写。
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
        // 节点名 → 地区（仅在能识别时给出；识别不到就不出现在表里）。
        // 地区由节点名推断（util::region），不依赖订阅内的额外字段，
        // 因此对既有订阅同样生效。
        let mut regions: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
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
                    for node in &all {
                        if let Some(region) = crate::util::region::detect_region(node) {
                            regions.insert(node.clone(), region.to_string());
                        }
                    }
                    groups.push(serde_json::json!({
                        "name": name,
                        "type": group_type,
                        "now": now,
                        "all": all,
                    }));
                }
            }
        }
        // 顶层节点（不隶属于任何组）也一并标注，供直连/手动节点使用。
        if let Some(proxies) = json.get("proxies").and_then(|v| v.as_object()) {
            for (name, value) in proxies {
                if regions.contains_key(name) {
                    continue;
                }
                let is_group = ["Selector", "URLTest", "Fallback", "LoadBalance", "Relay"]
                    .iter()
                    .any(|t| {
                        value
                            .get("type")
                            .and_then(|v| v.as_str())
                            .map(|x| t.eq_ignore_ascii_case(x))
                            .unwrap_or(false)
                    });
                if is_group {
                    continue;
                }
                if let Some(region) = crate::util::region::detect_region(name) {
                    regions.insert(name.clone(), region.to_string());
                }
            }
        }
        // regions 附在每个 group 上（前端按节点名查），避免额外 IPC 往返。
        if !regions.is_empty() {
            let regions_json = serde_json::to_value(&regions).unwrap_or(serde_json::Value::Null);
            for g in groups.iter_mut() {
                if let Some(obj) = g.as_object_mut() {
                    obj.insert("regions".to_string(), regions_json.clone());
                }
            }
        }
        Ok(groups)
    }

    /// 获取 /proxies 原始 JSON（含 history / alive / now / all 全量字段）。
    /// 与 `get_proxy_groups`（UI 裁剪版）不同，不裁剪任何字段——
    /// 供节点池巡检（core::patrol）消费测速历史做失活判定。
    pub(crate) async fn get_proxies_raw(&self) -> Result<serde_json::Value> {
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
        Ok(resp.json().await?)
    }

    /// 获取规则列表（GET /rules），只保留 type / payload / proxy 三个字段。
    ///
    /// mihomo 的规则条目还带 size、index、extra（命中统计）等字段；规则页首期只读展示，
    /// 在 Rust 侧裁剪可以减小 IPC 与 WebView 内存开销（规则表可达上万条）。
    pub(crate) async fn get_rules(&self) -> Result<Vec<serde_json::Value>> {
        let url = self.api_url(&["rules"], None)?;
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
        Ok(trim_rules(&json))
    }

    /// 选择代理组中的某个代理（PUT /proxies/{group}，组名 URL 编码）。
    ///
    /// 切换成功后按组排空旧连接（P0-4）：mihomo 不会迁移已建立的连接，
    /// 旧连接继续钉在原节点上——若原节点已死则半开挂死，表现为
    /// “切了节点仍然断网”。只排空经由该组的连接，不影响其他组
    /// （用户在“扶梯出行”切节点不应打断“影音视听”的下载）。
    /// UI 与托盘两条切换路径都经过这里，自动获得排空语义。
    pub(crate) async fn select_proxy_group(&self, group: String, mut proxy: String) -> Result<()> {
        // A refreshed subscription can rename a node or append a provider multiplier
        // suffix. Resolve a stale UI choice against the group's current node list.
        if let Ok(group_url) = self.api_url(&["proxies", &group], None) {
            if let Ok(response) = self
                .api_client
                .get(group_url)
                .headers(self.api_headers()?)
                .send()
                .await
            {
                if let Ok(value) = response.json::<serde_json::Value>().await {
                    if let Some(nodes) = value.get("all").and_then(|v| v.as_array()) {
                        let names: Vec<&str> = nodes.iter().filter_map(|v| v.as_str()).collect();
                        if !names.iter().any(|name| *name == proxy) {
                            if let Some(replacement) = fuzzy_proxy_match(&proxy, &names) {
                                info!(
                                    "Resolved stale proxy selection '{}' -> '{}'",
                                    proxy, replacement
                                );
                                proxy = replacement.to_string();
                            }
                        }
                    }
                }
            }
        }
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
            match self.close_group_connections(&group).await {
                Ok(n) if n > 0 => {
                    info!("Drained {} stale connection(s) of group '{}'", n, group)
                }
                Ok(_) => {}
                Err(e) => warn!(
                    "Failed to drain connections of group '{}' after switch: {}",
                    group, e
                ),
            }
            Ok(())
        } else {
            warn!("Failed to select proxy group: {}", resp.status());
            Err(Error::Other(format!(
                "Controller returned {}",
                resp.status()
            )))
        }
    }

    /// Clear fake-IP mappings and re-test the built-in url-test group after a
    /// network transition or a long system sleep (P1-2).
    ///
    /// The probe uses msftconnecttest.com — already allow-listed in fake-ip-filter —
    /// so it never introduces an extra external-domain dependency, and works more
    /// reliably on Windows Wi-Fi networks than gstatic.com.
    pub(crate) async fn recover_after_network_change(&self) -> Result<()> {
        let flush_url = self.api_url(&["cache", "fakeip", "flush"], None)?;
        let flush = self
            .api_client
            .delete(flush_url)
            .headers(self.api_headers()?)
            .send()
            .await?;
        if !flush.status().is_success() {
            warn!("Fake-IP cache flush returned {}", flush.status());
        }

        let group = crate::config::model::product_data::GROUP_AUTO;
        let group_url = self.api_url(&["proxies", group], None)?;
        let response = self
            .api_client
            .get(group_url)
            .headers(self.api_headers()?)
            .send()
            .await?;
        if !response.status().is_success() {
            return Ok(());
        }
        let value: serde_json::Value = response.json().await?;
        if value.get("type").and_then(|v| v.as_str()) != Some("URLTest")
            && value.get("type").and_then(|v| v.as_str()) != Some("url-test")
        {
            return Ok(());
        }
        let test_url = "https://www.msftconnecttest.com/connecttest.txt";
        let fake = {
            let cfg = self.config.read();
            crate::util::fetch::fake_ip_range_from_config(&cfg)
        };
        crate::util::fetch::validate_url_with(test_url, fake.as_ref()).await?;
        let query = [("url", test_url), ("timeout", "5000")];
        let delay_url = self.api_url(&["group", group, "delay"], Some(&query))?;
        let tested = self
            .api_client
            .get(delay_url)
            .headers(self.api_headers()?)
            .send()
            .await?;
        if !tested.status().is_success() {
            warn!("Automatic proxy-group retest returned {}", tested.status());
        }
        Ok(())
    }

    /// TUN enabled-after lightweight connectivity probe (diagnostic only, does
    /// not block the transaction).
    ///
    /// Probes the Windows NCSI probe domain via the built-in auto-select group:
    /// the domain is allow-listed in fake-ip-filter (real resolution), so one
    /// probe covers the whole chain "DNS resolution → route → node exit".
    /// Common TUN-break causes — auto-detect-interface picking a wrong adapter,
    /// DoH upstream blocked, captive portal not authenticated — all make this
    /// probe fail.
    ///
    /// Returns Ok(false) when the auto group is missing or not URLTest, meaning
    /// the probe cannot run; silently skipped in that case.
    pub(crate) async fn probe_tun_connectivity(&self) -> Result<bool> {
        let group = crate::config::model::product_data::GROUP_AUTO;
        let group_url = self.api_url(&["proxies", group], None)?;
        let response = self
            .api_client
            .get(group_url)
            .headers(self.api_headers()?)
            .send()
            .await?;
        if !response.status().is_success() {
            return Ok(false);
        }
        let value: serde_json::Value = response.json().await?;
        if value.get("type").and_then(|v| v.as_str()) != Some("URLTest")
            && value.get("type").and_then(|v| v.as_str()) != Some("url-test")
        {
            return Ok(false);
        }
        let test_url = "https://www.msftconnecttest.com/connecttest.txt";
        let fake = {
            let cfg = self.config.read();
            crate::util::fetch::fake_ip_range_from_config(&cfg)
        };
        crate::util::fetch::validate_url_with(test_url, fake.as_ref()).await?;
        let query = [("url", test_url), ("timeout", "5000")];
        let delay_url = self.api_url(&["group", group, "delay"], Some(&query))?;
        let tested = self
            .api_client
            .get(delay_url)
            .headers(self.api_headers()?)
            .send()
            .await?;
        Ok(tested.status().is_success())
    }

    /// 关闭经由指定组的全部活动连接（选择性排空，P0-4）。
    ///
    /// GET /connections 原始列表 → 过滤 `chains` 含该组 → 逐个
    /// DELETE /connections/{id}。返回关闭数；单个删除失败只记日志，
    /// 不影响其余（best-effort：排空是切换的增强而非前置条件）。
    pub(crate) async fn close_group_connections(&self, group: &str) -> Result<usize> {
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
        let Some(conns) = json.get("connections").and_then(|v| v.as_array()) else {
            return Ok(0);
        };

        let mut closed = 0usize;
        for conn in conns {
            let in_group = conn
                .get("chains")
                .and_then(|c| c.as_array())
                .map(|chains| chains.iter().any(|c| c.as_str() == Some(group)))
                .unwrap_or(false);
            if !in_group {
                continue;
            }
            let Some(id) = conn.get("id").and_then(|v| v.as_str()) else {
                continue;
            };
            let del = self.api_url(&["connections", id], None)?;
            match self
                .api_client
                .delete(del)
                .headers(self.api_headers()?)
                .send()
                .await
            {
                Ok(r) if r.status().is_success() => closed += 1,
                Ok(r) => warn!("DELETE /connections/{} returned {}", id, r.status()),
                Err(e) => warn!("DELETE /connections/{} failed: {}", id, e),
            }
        }
        Ok(closed)
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
        let fake = {
            let cfg = self.config.read();
            crate::util::fetch::fake_ip_range_from_config(&cfg)
        };
        crate::util::fetch::validate_url_with(&test_url, fake.as_ref()).await?;
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
        let total = json
            .get("connections")
            .and_then(|v| v.as_array())
            .map_or(0, Vec::len);
        if let Some(arr) = json.get("connections").and_then(|v| v.as_array()) {
            for conn in arr.iter().take(MAX_CONNECTIONS_RETURNED) {
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
        let truncated = total > MAX_CONNECTIONS_RETURNED;

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

/// 把 GET /rules 的响应裁剪为 `[{type, payload, proxy}]`。
/// 缺失 / 类型不符的字段回落为空字符串；`rules` 缺失或不是数组时返回空列表。
fn trim_rules(json: &serde_json::Value) -> Vec<serde_json::Value> {
    let str_field = |rule: &serde_json::Value, key: &str| -> String {
        rule.get(key)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    json.get("rules")
        .and_then(|v| v.as_array())
        .map(|rules| {
            rules
                .iter()
                .map(|rule| {
                    serde_json::json!({
                        "type": str_field(rule, "type"),
                        "payload": str_field(rule, "payload"),
                        "proxy": str_field(rule, "proxy"),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
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
    fn tun_patch_preserves_settings_and_overrides_enable() {
        let tun = serde_json::json!({ "enable": false, "stack": "system", "mtu": 1400 });
        let enabled = tun_patch_payload(&tun, true).unwrap();
        assert_eq!(enabled["tun"]["enable"], true);
        assert_eq!(enabled["tun"]["stack"], "system");
        assert_eq!(enabled["tun"]["mtu"], 1400);

        let disabled = tun_patch_payload(&tun, false).unwrap();
        assert_eq!(disabled["tun"]["enable"], false);
    }

    #[test]
    fn tun_enable_parser_defaults_missing_or_malformed_state_to_false() {
        assert!(parse_tun_enable(
            &serde_json::json!({ "tun": { "enable": true } })
        ));
        assert!(!parse_tun_enable(
            &serde_json::json!({ "tun": { "enable": false } })
        ));
        assert!(!parse_tun_enable(&serde_json::json!({ "tun": {} })));
        assert!(!parse_tun_enable(
            &serde_json::json!({ "tun": { "enable": "true" } })
        ));
    }

    #[test]
    fn controller_pipe_name_is_stable_secret_derived_and_redacted() {
        let first = controller_pipe_name("install-secret-one");
        assert_eq!(first, controller_pipe_name("install-secret-one"));
        assert_ne!(first, controller_pipe_name("install-secret-two"));
        assert!(first.starts_with(r"\\.\pipe\ClashEdge-Mihomo-Controller-"));
        assert!(!first.contains("install-secret-one"));
    }

    #[test]
    fn fuzzy_proxy_match_strips_multiplier_and_handles_small_name_changes() {
        let candidates = ["🇺🇸 US Los Angeles 03 x1.20", "🇺🇸 US Los Angeles 30 x1.2"];
        assert_eq!(
            fuzzy_proxy_match("🇺🇸 US Los Angeles 03", &candidates),
            Some("🇺🇸 US Los Angeles 03 x1.20")
        );
        assert_eq!(strip_multiplier_suffix("relay ×1.5"), "relay");
    }

    #[test]
    fn fuzzy_proxy_match_rejects_ambiguous_or_distant_names() {
        assert_eq!(
            fuzzy_proxy_match("US node 1", &["US node 2", "US node 3"]),
            None
        );
        assert_eq!(fuzzy_proxy_match("old", &["a very different node"]), None);
        assert_eq!(fuzzy_proxy_match("US", &["JP"]), None);
    }

    #[test]
    fn trim_rules_keeps_only_type_payload_proxy() {
        let json = serde_json::json!({
            "rules": [
                {
                    "index": 0,
                    "type": "DomainSuffix",
                    "payload": "openai.com",
                    "proxy": "人工智能",
                    "size": -1,
                    "extra": { "hitCount": 3, "missCount": 1 }
                },
                { "type": "Match", "payload": "", "proxy": "DIRECT" }
            ]
        });
        let rules = trim_rules(&json);
        assert_eq!(rules.len(), 2);
        assert_eq!(
            rules[0],
            serde_json::json!({
                "type": "DomainSuffix",
                "payload": "openai.com",
                "proxy": "人工智能",
            })
        );
        assert_eq!(rules[0].as_object().unwrap().len(), 3);
        assert_eq!(rules[1]["type"], "Match");
        assert_eq!(rules[1]["payload"], "");
    }

    #[test]
    fn trim_rules_handles_empty_and_malformed_responses() {
        assert!(trim_rules(&serde_json::json!({ "rules": [] })).is_empty());
        assert!(trim_rules(&serde_json::json!({})).is_empty());
        assert!(trim_rules(&serde_json::json!({ "rules": "nope" })).is_empty());
        // 缺字段 / 类型不符：回落为空字符串而不是 panic
        let rules = trim_rules(&serde_json::json!({ "rules": [ { "type": 7 }, {} ] }));
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0]["type"], "");
        assert_eq!(rules[1]["proxy"], "");
    }

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
