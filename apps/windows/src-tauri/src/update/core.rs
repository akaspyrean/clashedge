//! Independent signed Mihomo core update channel.
//!
//! Core updates are published as versioned `mihomo-core-vX.Y.Z` GitHub releases,
//! separate from the portable app release. The current app key signs a manifest
//! containing the raw executable's SHA-256; verified binaries are staged in a
//! versioned Data directory and selected after the next app restart.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::Manager;
use tracing::info;

use super::{parse_version, verify_manifest_signature, UpdateManifest, UPDATE_PUBLIC_KEY};
use crate::core::integrity::CORE_DOWNLOAD_URL_PREFIX;
use crate::util::error::{Error, Result};

const RELEASES_API: &str =
    "https://api.github.com/repos/akaspyrean/clashedge/releases?per_page=100";
const CORE_ASSET_NAME: &str = "mihomo-core-win-x64.exe";
const CORE_MANIFEST_NAME: &str = "mihomo-core-manifest.json";
const MAX_RELEASES_BYTES: usize = 2 * 1024 * 1024;
const MAX_METADATA_BYTES: usize = 64 * 1024;
const MAX_SIGNATURE_BYTES: usize = 4 * 1024;
const MAX_CORE_BYTES: u64 = 200 * 1024 * 1024;
const DOWNLOAD_DEADLINE: std::time::Duration = std::time::Duration::from_secs(900);
const VERSION_CHECK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct CoreUpdateStatus {
    pub status: &'static str,
    pub current: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StagedCoreUpdate {
    pub version: String,
    pub sha256: String,
    pub restart_required: bool,
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    draft: bool,
    assets: Vec<GithubAsset>,
}

#[derive(Debug, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

struct VerifiedCoreManifest {
    manifest: UpdateManifest,
    raw: Vec<u8>,
    signature: String,
}

/// Whether this build contains the public key required by the core channel.
pub fn chain_ready() -> bool {
    !UPDATE_PUBLIC_KEY.trim().is_empty()
}

pub async fn check_for_core_update(app: &tauri::AppHandle) -> Result<CoreUpdateStatus> {
    if !chain_ready() {
        return Err(Error::Other(
            "Mihomo core updates are unavailable: the signing public key is not embedded"
                .to_string(),
        ));
    }
    let current = current_core_version(app).await?;
    let latest = fetch_latest_core_manifest(app).await?;
    if crate::core::integrity::is_newer_version(&latest.manifest.version, &current) {
        Ok(CoreUpdateStatus {
            status: "available",
            current,
            version: Some(latest.manifest.version),
        })
    } else {
        Ok(CoreUpdateStatus {
            status: "up_to_date",
            current,
            version: None,
        })
    }
}

pub async fn download_core_update(app: &tauri::AppHandle) -> Result<StagedCoreUpdate> {
    if !chain_ready() {
        return Err(Error::Other(
            "Mihomo core updates are unavailable: the signing public key is not embedded"
                .to_string(),
        ));
    }

    let current = current_core_version(app).await?;
    let verified = fetch_latest_core_manifest(app).await?;
    let version = verified.manifest.version.as_str();
    if !crate::core::integrity::is_newer_version(version, &current) {
        return Err(Error::Other(format!(
            "Mihomo core {} is not newer than the installed core {}",
            version, current
        )));
    }

    let data_dir = crate::util::paths::get_app_data_dir(app)?;
    let core_dir = crate::core::integrity::updated_core_dir(&data_dir, version);
    std::fs::create_dir_all(&core_dir)?;
    let binary_path = core_dir.join("mihomo-win64.exe");
    let partial_path = core_dir.join("mihomo-win64.exe.part");
    if binary_path.exists() {
        crate::core::integrity::verify_core_binary(&binary_path, &data_dir)?;
        return Ok(StagedCoreUpdate {
            version: version.to_string(),
            sha256: verified.manifest.sha256,
            restart_required: true,
        });
    }

    crate::util::atomic::atomic_write(&core_dir.join(CORE_MANIFEST_NAME), &verified.raw)?;
    crate::util::atomic::atomic_write(
        &core_dir.join(format!("{}.minisig", CORE_MANIFEST_NAME)),
        verified.signature.as_bytes(),
    )?;

    let response =
        crate::util::fetch::get_direct_first_streaming(app, &verified.manifest.url).await?;
    if !response.status().is_success() {
        return Err(Error::Other(format!(
            "Mihomo core download failed: HTTP {}",
            response.status()
        )));
    }
    if response
        .content_length()
        .is_some_and(|size| size > MAX_CORE_BYTES)
    {
        return Err(Error::Other(
            "Mihomo core download exceeds the size limit".to_string(),
        ));
    }

    let mut response = response;
    let partial_for_download = partial_path.clone();
    let download = tokio::time::timeout(DOWNLOAD_DEADLINE, async move {
        let mut file = tokio::fs::File::create(&partial_for_download).await?;
        let mut digest = Sha256::new();
        let mut total = 0u64;
        while let Some(chunk) = response.chunk().await? {
            total = total.saturating_add(chunk.len() as u64);
            if total > MAX_CORE_BYTES {
                return Err(Error::Other(
                    "Mihomo core download exceeds the size limit".to_string(),
                ));
            }
            digest.update(&chunk);
            tokio::io::AsyncWriteExt::write_all(&mut file, &chunk).await?;
        }
        tokio::io::AsyncWriteExt::flush(&mut file).await?;
        file.sync_all().await?;
        Ok::<_, Error>(format!("{:x}", digest.finalize()))
    })
    .await;
    let downloaded_hash = match download {
        Ok(Ok(hash)) => hash,
        Ok(Err(e)) => {
            let _ = std::fs::remove_file(&partial_path);
            return Err(e);
        }
        Err(_) => {
            let _ = std::fs::remove_file(&partial_path);
            return Err(Error::Other("Mihomo core download timed out".to_string()));
        }
    };

    if !downloaded_hash.eq_ignore_ascii_case(&verified.manifest.sha256) {
        let _ = std::fs::remove_file(&partial_path);
        return Err(Error::Other(format!(
            "Mihomo core SHA-256 mismatch: expected {}, got {}",
            verified.manifest.sha256, downloaded_hash
        )));
    }

    // The signed hash is checked before executing even the version probe.
    crate::core::integrity::verify_core_binary(&partial_path, &data_dir)?;
    let mut version_command = tokio::process::Command::new(&partial_path);
    version_command.arg("-v").kill_on_drop(true);
    #[cfg(target_os = "windows")]
    version_command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let output = tokio::time::timeout(VERSION_CHECK_TIMEOUT, version_command.output())
        .await
        .map_err(|_| Error::Other("Mihomo core version check timed out".to_string()))??;
    if !output.status.success() {
        let _ = std::fs::remove_file(&partial_path);
        return Err(Error::Other(
            "Mihomo core failed its version check".to_string(),
        ));
    }
    let version_output = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let actual_version = extract_version(&version_output)
        .ok_or_else(|| Error::Other("Mihomo core reported an unreadable version".to_string()))?;
    if actual_version != version {
        let _ = std::fs::remove_file(&partial_path);
        return Err(Error::Other(format!(
            "Downloaded Mihomo core reports v{}, expected v{}",
            actual_version, version
        )));
    }

    std::fs::rename(&partial_path, &binary_path)?;
    crate::core::integrity::verify_core_binary(&binary_path, &data_dir)?;
    info!(
        "Verified Mihomo core {} staged for the next app restart",
        version
    );
    Ok(StagedCoreUpdate {
        version: version.to_string(),
        sha256: downloaded_hash,
        restart_required: true,
    })
}

pub async fn staged_core_update(app: &tauri::AppHandle) -> Result<Option<StagedCoreUpdate>> {
    let current = current_core_version(app).await?;
    let data_dir = crate::util::paths::get_app_data_dir(app)?;
    for binary in crate::core::integrity::updated_core_candidates(&data_dir) {
        let version = binary
            .parent()
            .and_then(std::path::Path::file_name)
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        if !crate::core::integrity::is_newer_version(version, &current) {
            continue;
        }
        if crate::core::integrity::verify_core_binary(&binary, &data_dir).is_ok() {
            let sha256 = std::fs::read(binary.parent().unwrap().join(CORE_MANIFEST_NAME))
                .ok()
                .and_then(|raw| serde_json::from_slice::<UpdateManifest>(&raw).ok())
                .map(|manifest| manifest.sha256)
                .unwrap_or_default();
            return Ok(Some(StagedCoreUpdate {
                version: version.to_string(),
                sha256,
                restart_required: true,
            }));
        }
    }
    Ok(None)
}

async fn current_core_version(app: &tauri::AppHandle) -> Result<String> {
    let state = app.state::<crate::AppState>();
    let core = state
        .core_manager
        .get()
        .ok_or_else(|| Error::InvalidState("Core not initialized".to_string()))?;
    let output = core.version().await?;
    extract_version(&output)
        .ok_or_else(|| Error::Other(format!("unable to parse Mihomo version from {:?}", output)))
}

async fn fetch_latest_core_manifest(app: &tauri::AppHandle) -> Result<VerifiedCoreManifest> {
    let releases_bytes = fetch_small(app, RELEASES_API, MAX_RELEASES_BYTES).await?;
    let releases: Vec<GithubRelease> = serde_json::from_slice(&releases_bytes)
        .map_err(|e| Error::Other(format!("invalid GitHub releases response: {}", e)))?;
    let (version, release) = releases
        .into_iter()
        .filter(|release| !release.draft)
        .filter_map(|release| {
            let version = release.tag_name.strip_prefix("mihomo-core-v")?;
            let parsed = parse_version(version)?;
            Some((parsed, version.to_string(), release))
        })
        .max_by_key(|(parsed, _, _)| *parsed)
        .map(|(_, version, release)| (version, release))
        .ok_or_else(|| Error::Other("no published Mihomo core release was found".to_string()))?;

    let expected_prefix = format!("{}{}", CORE_DOWNLOAD_URL_PREFIX, version);
    let expected_asset_url = format!("{}/{CORE_ASSET_NAME}", expected_prefix);
    let expected_manifest_url = format!("{}/{CORE_MANIFEST_NAME}", expected_prefix);
    let expected_signature_url = format!("{}.minisig", expected_manifest_url);
    let expected_signature_name = format!("{}.minisig", CORE_MANIFEST_NAME);
    for (name, url) in [
        (CORE_ASSET_NAME, expected_asset_url.as_str()),
        (CORE_MANIFEST_NAME, expected_manifest_url.as_str()),
        (
            expected_signature_name.as_str(),
            expected_signature_url.as_str(),
        ),
    ] {
        if !release
            .assets
            .iter()
            .any(|asset| asset.name == name && asset.browser_download_url == url)
        {
            return Err(Error::Other(format!(
                "Mihomo core release {} is missing expected asset {}",
                version, name
            )));
        }
    }

    crate::util::fetch::validate_url_app(app, &expected_manifest_url).await?;
    crate::util::fetch::validate_url_app(app, &expected_signature_url).await?;
    let (raw, signature) = tokio::try_join!(
        fetch_small(app, &expected_manifest_url, MAX_METADATA_BYTES),
        fetch_small(app, &expected_signature_url, MAX_SIGNATURE_BYTES),
    )?;
    let signature = String::from_utf8(signature)
        .map_err(|e| Error::Other(format!("invalid Mihomo signature text: {}", e)))?;
    verify_manifest_signature(UPDATE_PUBLIC_KEY, &raw, &signature)?;

    let manifest: UpdateManifest = serde_json::from_slice(&raw)
        .map_err(|e| Error::Other(format!("invalid signed Mihomo manifest: {}", e)))?;
    if manifest.version != version
        || manifest.url != expected_asset_url
        || manifest.sha256.len() != 64
        || !manifest.sha256.chars().all(|c| c.is_ascii_hexdigit())
    {
        return Err(Error::Other(
            "signed Mihomo manifest has an invalid version, URL, or SHA-256".to_string(),
        ));
    }
    crate::util::fetch::validate_url_app(app, &manifest.url).await?;
    Ok(VerifiedCoreManifest {
        manifest,
        raw,
        signature,
    })
}

async fn fetch_small(app: &tauri::AppHandle, url: &str, limit: usize) -> Result<Vec<u8>> {
    let response = crate::util::fetch::get_direct_first(app, url).await?;
    if !response.status().is_success() {
        return Err(Error::Other(format!(
            "Mihomo update request failed: HTTP {}",
            response.status()
        )));
    }
    if response
        .content_length()
        .is_some_and(|size| size > limit as u64)
    {
        return Err(Error::Other(
            "Mihomo update metadata exceeds size limits".to_string(),
        ));
    }
    let body = response.bytes().await?;
    if body.len() > limit {
        return Err(Error::Other(
            "Mihomo update metadata exceeds size limits".to_string(),
        ));
    }
    Ok(body.to_vec())
}

fn extract_version(text: &str) -> Option<String> {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '.' || c == '-'))
        .find(|token| parse_version(token).is_some())
        .map(|token| token.trim_start_matches('v').to_string())
}
