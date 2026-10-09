// src-tauri/src/util/region.rs
//! 节点区域检测编排：解析节点 server 地址 → GeoIP 查国家 ISO 代码
//! → 写入 profiles/<name>.region.json 旁文件。
//!
//! 在订阅导入/刷新归一化后调用一次（网络/磁盘耗时不占构建关键路径）。
//! 失败只降级为"无区域信息"，不阻断导入/刷新本身——
//! build_runtime_config 在无 region.json 时退化为当前行为（全节点注入）。

use std::collections::HashMap;
use std::net::IpAddr;
use std::path::PathBuf;

use serde::Serialize;
use tauri::AppHandle;
use tracing::{debug, warn};

use crate::geodata::geoip_lookup::GeoIpIndex;
use crate::util::error::Result;
use crate::util::paths::get_profiles_dir;

/// region.json 侧文件格式
#[derive(Serialize)]
struct RegionFile {
    /// 节点名 → ISO 国家代码（如 US / HK / JP）
    regions: HashMap<String, String>,
}

/// 解析后的节点 server 信息
#[derive(Clone)]
pub(super) struct NodeServerInfo {
    pub name: String,
    pub server: String,
}

/// 从归一化节点列表提取 (name, server) 对
pub(super) fn extract_servers(proxies: &[serde_yaml::Value]) -> Vec<NodeServerInfo> {
    proxies
        .iter()
        .filter_map(|p| {
            let name = p.get("name").and_then(|n| n.as_str())?;
            let server = p.get("server").and_then(|s| s.as_str())?;
            Some(NodeServerInfo {
                name: name.to_string(),
                server: server.trim().to_string(),
            })
        })
        .collect()
}

/// 对唯一 server 列表解析出 IP 地址（IP 字面量直接用；域名走 DoH）
async fn resolve_servers(servers: Vec<String>) -> HashMap<String, Vec<IpAddr>> {
    let mut out: HashMap<String, Vec<IpAddr>> = HashMap::new();
    for server in servers {
        if let Ok(ip) = server.parse::<IpAddr>() {
            out.insert(server, vec![ip]);
            continue;
        }
        // 域名 → DoH 解析
        match crate::util::fetch::resolve_real_addrs(&server).await {
            Ok(addrs) if !addrs.is_empty() => {
                out.insert(server, addrs);
            }
            Ok(_) => {
                debug!("Region: no addresses resolved for {}", server);
            }
            Err(e) => {
                debug!("Region: DoH failed for {}: {}", server, e);
            }
        }
    }
    out
}

/// 检测节点区域并持久化到 region.json；返回 (节点名→ISO代码) 映射。
/// 任何 GeoIP 加载失败/DoH 失败/写盘失败都只降级为空 map，不向上抛错。
pub async fn detect_and_persist(
    app: &AppHandle,
    profile_name: &str,
    proxies: &[serde_yaml::Value],
) -> HashMap<String, String> {
    let servers = extract_servers(proxies);
    if servers.is_empty() {
        return HashMap::new();
    }

    // 加载 GeoIP.dat（17MB, ~100-300ms）
    let geoip_path: Option<PathBuf> = crate::util::paths::get_geoip_path(app).ok();
    let index: Option<GeoIpIndex> = match geoip_path.as_deref() {
        Some(path) => {
            let path = path.to_path_buf();
            match tokio::task::spawn_blocking(move || GeoIpIndex::load(&path)).await {
                Ok(Ok(idx)) => Some(idx),
                Ok(Err(e)) => {
                    debug!("Region: GeoIP.dat unavailable: {}", e);
                    None
                }
                Err(e) => {
                    debug!("Region: GeoIP load task panicked: {}", e);
                    None
                }
            }
        }
        None => {
            debug!("Region: GeoIP.dat path not resolvable");
            None
        }
    };

    let index = match index {
        Some(i) => i,
        None => {
            // 无 GeoIP 数据 → 无法检测区域，返回空 map（调用方按无区域处理）
            return HashMap::new();
        }
    };

    // 去重后批量解析 server → IP
    let unique_servers: Vec<String> = {
        let mut seen = std::collections::HashSet::new();
        servers
            .iter()
            .filter(|s| seen.insert(s.server.clone()))
            .map(|s| s.server.clone())
            .collect()
    };

    let ip_map = resolve_servers(unique_servers).await;

    // 构建 region 映射
    let mut regions: HashMap<String, String> = HashMap::new();
    for info in &servers {
        if let Some(ips) = ip_map.get(&info.server) {
            // 取第一个可定位的 IP
            for ip in ips {
                if let Some(code) = index.lookup(*ip) {
                    regions.insert(info.name.clone(), code.to_string());
                    break;
                }
            }
        }
    }

    // 写入 region.json
    if let Err(e) = persist_region_map(app, profile_name, &regions) {
        warn!(
            "Region: failed to persist region.json for {}: {}",
            profile_name, e
        );
    }

    regions
}

/// 写入 profiles/<name>.region.json
fn persist_region_map(
    app: &AppHandle,
    profile_name: &str,
    regions: &HashMap<String, String>,
) -> Result<()> {
    let profiles_dir = get_profiles_dir(app)?;
    let file = profiles_dir.join(format!("{}.region.json", profile_name));
    let data = RegionFile {
        regions: regions.clone(),
    };
    let json = serde_json::to_vec(&data)?;
    crate::util::atomic::atomic_write(&file, &json)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_servers_from_yaml() {
        let proxies = vec![
            serde_yaml::from_str::<serde_yaml::Value>(
                "name: A\ntype: ss\nserver: 1.2.3.4\nport: 8388",
            )
            .unwrap(),
            serde_yaml::from_str::<serde_yaml::Value>(
                "name: B\ntype: ss\nserver: us1.example.com\nport: 443",
            )
            .unwrap(),
        ];
        let infos = extract_servers(&proxies);
        assert_eq!(infos.len(), 2);
        assert_eq!(infos[0].server, "1.2.3.4");
        assert_eq!(infos[1].name, "B");
    }
}
