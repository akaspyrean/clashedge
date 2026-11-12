//! Runtime integrity checks for the Mihomo executable.
//!
//! The bundled core is trusted only when its SHA-256 matches the repository's
//! pinned `assets.lock.json` value. A core installed through the independent
//! update channel is trusted only when its manifest signature and binary hash
//! both verify with the same public key used by the portable app updater.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::util::error::{Error, Result};

const ASSET_LOCK: &str = include_str!("../../../../../assets.lock.json");
const UPDATED_CORE_DIR: &str = "core-updates";
const CORE_BINARY_NAME: &str = "mihomo-win64.exe";
const CORE_MANIFEST_NAME: &str = "mihomo-core-manifest.json";
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
const MAX_SIGNATURE_BYTES: u64 = 4 * 1024;
pub const CORE_DOWNLOAD_URL_PREFIX: &str =
    "https://github.com/akaspyrean/clashedge/releases/download/mihomo-core-v";

/// Directory containing independently updated, versioned core installations.
pub fn updated_core_dir(data_dir: &Path, version: &str) -> PathBuf {
    data_dir
        .join(UPDATED_CORE_DIR)
        .join(safe_version_component(version))
}

pub fn updated_core_binary_path(data_dir: &Path, version: &str) -> PathBuf {
    updated_core_dir(data_dir, version).join(CORE_BINARY_NAME)
}

/// Return installed update candidates in descending semantic-version order.
pub fn updated_core_candidates(data_dir: &Path) -> Vec<PathBuf> {
    let root = data_dir.join(UPDATED_CORE_DIR);
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut candidates: Vec<(CoreVersion, PathBuf)> = entries
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| {
            let version = entry.file_name().to_string_lossy().into_owned();
            let parsed = parse_version(&version)?;
            let binary = entry.path().join(CORE_BINARY_NAME);
            binary.is_file().then_some((parsed, binary))
        })
        .collect();
    candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.0));
    candidates.into_iter().map(|(_, path)| path).collect()
}

/// Check a binary immediately before executing it.
pub fn verify_core_binary(path: &Path, data_dir: &Path) -> Result<()> {
    let expected_hash = if is_updated_core(path, data_dir) {
        signed_updated_core_hash(path.parent().ok_or_else(|| {
            Error::Other("updated Mihomo binary has no parent directory".to_string())
        })?)?
    } else {
        bundled_core_hash()?
    };
    let actual_hash = sha256_file(path)?;
    if !actual_hash.eq_ignore_ascii_case(&expected_hash) {
        return Err(Error::Other(format!(
            "mihomo integrity check failed for {}: expected SHA-256 {}, got {}",
            path.display(),
            expected_hash,
            actual_hash
        )));
    }
    Ok(())
}

fn is_updated_core(path: &Path, data_dir: &Path) -> bool {
    let root = data_dir.join(UPDATED_CORE_DIR);
    path.parent()
        .is_some_and(|parent| parent.starts_with(&root))
}

fn signed_updated_core_hash(core_dir: &Path) -> Result<String> {
    let manifest_path = core_dir.join(CORE_MANIFEST_NAME);
    let signature_path = core_dir.join(format!("{}.minisig", CORE_MANIFEST_NAME));
    let manifest_size = std::fs::metadata(&manifest_path)?.len();
    let signature_size = std::fs::metadata(&signature_path)?.len();
    if manifest_size > MAX_MANIFEST_BYTES || signature_size > MAX_SIGNATURE_BYTES {
        return Err(Error::Other(
            "mihomo update metadata exceeds size limits".to_string(),
        ));
    }

    let manifest_bytes = std::fs::read(&manifest_path)?;
    let signature = std::fs::read_to_string(&signature_path)?;
    crate::update::verify_manifest_signature(
        crate::update::UPDATE_PUBLIC_KEY,
        &manifest_bytes,
        &signature,
    )?;

    let manifest: crate::update::UpdateManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|e| Error::Other(format!("invalid signed mihomo manifest: {}", e)))?;
    let directory_version = core_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if manifest.version != directory_version
        || manifest.sha256.len() != 64
        || !manifest.sha256.chars().all(|c| c.is_ascii_hexdigit())
        || !manifest.url.starts_with(CORE_DOWNLOAD_URL_PREFIX)
    {
        return Err(Error::Other(
            "signed mihomo manifest has an invalid version, hash, or release URL".to_string(),
        ));
    }
    Ok(manifest.sha256)
}

pub fn bundled_core_version() -> Result<String> {
    let (version, _) = bundled_core_metadata()?;
    Ok(version)
}

/// Compare numeric stable versions. Non-semantic versions are never considered newer.
pub fn is_newer_version(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(candidate), Some(current)) => candidate > current,
        _ => false,
    }
}

fn bundled_core_hash() -> Result<String> {
    let (_, hash) = bundled_core_metadata()?;
    Ok(hash)
}

fn bundled_core_metadata() -> Result<(String, String)> {
    let lock: serde_json::Value = serde_json::from_str(ASSET_LOCK)
        .map_err(|e| Error::Other(format!("invalid embedded assets.lock.json: {}", e)))?;
    let assets = lock
        .get("assets")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| Error::Other("embedded assets.lock.json has no assets list".to_string()))?;
    let asset = assets
        .iter()
        .find(|asset| asset.get("name").and_then(serde_json::Value::as_str) == Some("mihomo"))
        .ok_or_else(|| Error::Other("embedded assets.lock.json has no Mihomo entry".to_string()))?;
    let version = asset
        .get("version")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| Error::Other("Mihomo entry has no version".to_string()))?;
    let hash = asset
        .get("extracted_sha256")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| Error::Other("Mihomo entry has no extracted SHA-256".to_string()))?;
    if parse_version(version).is_none()
        || hash.len() != 64
        || !hash.chars().all(|c| c.is_ascii_hexdigit())
    {
        return Err(Error::Other(
            "Mihomo version or pinned SHA-256 in assets.lock.json is invalid".to_string(),
        ));
    }
    Ok((version.to_string(), hash.to_string()))
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(hex_encode(&digest.finalize()))
}

/// Lower-case hexadecimal encoding of raw bytes (sha2 output has no `LowerHex`).
pub fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn safe_version_component(version: &str) -> String {
    version
        .chars()
        .take(32)
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct CoreVersion(u64, u64, u64);

fn parse_version(version: &str) -> Option<CoreVersion> {
    let version = version.strip_prefix('v').unwrap_or(version);
    let numeric = version.split(['-', '+']).next()?;
    let mut parts = numeric.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    parts
        .next()
        .is_none()
        .then_some(CoreVersion(major, minor, patch))
}
