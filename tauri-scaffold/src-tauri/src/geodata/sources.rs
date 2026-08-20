// src-tauri/src/geodata/sources.rs
//! GeoX/GeoIP/GeoSite URL configuration
//!
//! This module centralizes the URLs used for downloading GeoIP and GeoSite data files.
//! The URLs are configured to fetch from Mihomo/MetaCubeX release assets.

use serde::{Deserialize, Serialize};

/// Structure representing GeoIP/GeoSite download sources
///
/// Each variant contains a list of URLs from which the data file can be downloaded.
/// Multiple URLs are provided for fallback - if one source fails, the next is tried.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct GeoSources {
    /// GeoIP data file download URLs
    ///
    /// These URLs point to GeoIP.dat files compiled from IP-to-location mappings.
    /// The first URL is the primary source, subsequent ones are fallbacks.
    #[serde(default)]
    pub geoip: Vec<String>,

    /// GeoSite data file download URLs
    ///
    /// These URLs point to GeoSite.dat files containing domain-to-rule mappings.
    /// The first URL is the primary source, subsequent ones are fallbacks.
    #[serde(default)]
    pub geosite: Vec<String>,

    /// GeoX (mixed) data file download URLs
    ///
    /// These URLs point to a combined/merged GeoX file that may contain both
    /// GeoIP and GeoSite data in one file, or a specific format used by Mihomo.
    #[serde(default)]
    pub geox: Vec<String>,
}

impl GeoSources {
    /// Create a new GeoSources instance with default URLs
    ///
    /// Defaults to the standard MetaCubeX/Mihomo release URLs for GeoIP and GeoSite.
    pub fn new() -> Self {
        Self {
            geoip: vec![
                "https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/geoip.dat"
                    .to_string(),
                "https://cdn.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@release/geoip.dat"
                    .to_string(),
            ],
            geosite: vec![
                "https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/geosite.dat"
                    .to_string(),
                "https://cdn.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@release/geosite.dat"
                    .to_string(),
            ],
            geox: vec![
                "https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/geox.dat"
                    .to_string(),
            ],
        }
    }

    /// Get the primary URL for GeoIP download
    pub fn geoip_primary(&self) -> Option<&str> {
        self.geoip.first().map(|s| s.as_str())
    }

    /// Get the primary URL for GeoSite download
    pub fn geosite_primary(&self) -> Option<&str> {
        self.geosite.first().map(|s| s.as_str())
    }
}

/// `Default` 返回带真实默认 URL 的实例
/// （`new()` 与 `default()` 等价，二者都含真实 URL，供 updater 兜底使用）
impl Default for GeoSources {
    fn default() -> Self {
        Self::new()
    }
}
