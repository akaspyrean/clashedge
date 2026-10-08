// src-tauri/src/geodata/download.rs
//! GeoData / 规则集共用的受限下载助手：流式落盘 + 同步计算 SHA256 + 大小上限 +
//! 整体 deadline。所有请求都走 `util::fetch`（SSRF 防护、直连优先 + 本地代理兜底）。

use std::path::Path;
use std::time::Duration;

use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::util::error::{Error, Result};

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// 文件 SHA256（小写十六进制）；文件不存在 / 读取失败返回 None。
pub fn sha256_of_file(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Some(hex(&h.finalize()))
}

/// 下载小文件（清单 / 签名）到内存，超过 `max` 字节即中止。
pub async fn fetch_small(app: &tauri::AppHandle, url: &str, max: u64) -> Result<Vec<u8>> {
    let mut resp = crate::util::fetch::get_direct_first(app, url).await?;
    if !resp.status().is_success() {
        return Err(Error::Other(format!(
            "HTTP {} for {}",
            resp.status(),
            crate::util::fetch::redact_url_for_log(url)
        )));
    }
    let mut out: Vec<u8> = Vec::new();
    while let Some(chunk) = resp.chunk().await? {
        out.extend_from_slice(&chunk);
        if out.len() as u64 > max {
            return Err(Error::Other(format!("response exceeds {} bytes", max)));
        }
    }
    Ok(out)
}

/// 流式下载到 `dest`，返回 `(字节数, sha256)`。任何失败都会删除 `dest`。
pub async fn download_to_file(
    app: &tauri::AppHandle,
    url: &str,
    dest: &Path,
    max_bytes: u64,
    deadline: Duration,
) -> Result<(u64, String)> {
    let result = tokio::time::timeout(deadline, download_inner(app, url, dest, max_bytes)).await;
    let out = match result {
        Ok(r) => r,
        Err(_) => Err(Error::Other(format!(
            "download exceeded {}s deadline",
            deadline.as_secs()
        ))),
    };
    if out.is_err() {
        let _ = tokio::fs::remove_file(dest).await;
    }
    out
}

async fn download_inner(
    app: &tauri::AppHandle,
    url: &str,
    dest: &Path,
    max_bytes: u64,
) -> Result<(u64, String)> {
    let mut resp = crate::util::fetch::get_direct_first_streaming(app, url).await?;
    if !resp.status().is_success() {
        return Err(Error::Other(format!("HTTP {}", resp.status())));
    }
    if let Some(len) = resp.content_length() {
        if len > max_bytes {
            return Err(Error::Other(format!(
                "content-length {} exceeds limit {}",
                len, max_bytes
            )));
        }
    }
    let mut file = tokio::fs::File::create(dest).await?;
    let mut hasher = Sha256::new();
    let mut total: u64 = 0;
    while let Some(chunk) = resp.chunk().await? {
        total += chunk.len() as u64;
        if total > max_bytes {
            return Err(Error::Other(format!(
                "download exceeds {} MB limit",
                max_bytes / 1024 / 1024
            )));
        }
        hasher.update(&chunk);
        file.write_all(&chunk).await?;
    }
    file.flush().await?;
    file.sync_all().await?;
    if total == 0 {
        return Err(Error::Other("downloaded file is empty".to_string()));
    }
    Ok((total, hex(&hasher.finalize())))
}

/// 事务式替换一组文件：每个目标先备份为 `<name>.backup`，再 rename 新文件就位；
/// 任一步失败则把已替换的目标全部恢复，返回 Err。成功后清理备份。
/// `pairs` 为 `(新文件, 目标文件)`。
pub fn replace_all(pairs: &[(std::path::PathBuf, std::path::PathBuf)]) -> Result<()> {
    let mut done: Vec<(std::path::PathBuf, Option<std::path::PathBuf>)> = Vec::new();
    for (new, target) in pairs {
        let backup = {
            let mut n = target.file_name().unwrap_or_default().to_os_string();
            n.push(".backup");
            target.with_file_name(n)
        };
        let had = target.exists();
        let step = (|| -> std::io::Result<()> {
            if had {
                let _ = std::fs::remove_file(&backup);
                std::fs::rename(target, &backup)?;
            }
            std::fs::rename(new, target)?;
            Ok(())
        })();
        match step {
            Ok(()) => done.push((target.clone(), had.then_some(backup))),
            Err(e) => {
                // 当前目标：若备份已移走而新文件未就位，先恢复它
                let cur_backup = {
                    let mut n = target.file_name().unwrap_or_default().to_os_string();
                    n.push(".backup");
                    target.with_file_name(n)
                };
                if had && !target.exists() && cur_backup.exists() {
                    let _ = std::fs::rename(&cur_backup, target);
                }
                // 已完成的目标：回滚
                for (t, b) in done.iter().rev() {
                    if let Some(b) = b {
                        let _ = std::fs::remove_file(t);
                        let _ = std::fs::rename(b, t);
                    }
                }
                for (n, _) in pairs {
                    let _ = std::fs::remove_file(n);
                }
                return Err(Error::Other(format!(
                    "failed to replace {}: {} (all changes rolled back)",
                    target.display(),
                    e
                )));
            }
        }
    }
    for (_, b) in done {
        if let Some(b) = b {
            let _ = std::fs::remove_file(b);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("clashedge-dl-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn replace_all_swaps_and_cleans_backups() {
        let d = scratch("ok");
        std::fs::write(d.join("a.dat"), "old-a").unwrap();
        std::fs::write(d.join("a.new"), "new-a").unwrap();
        std::fs::write(d.join("b.new"), "new-b").unwrap(); // b 原本不存在
        replace_all(&[
            (d.join("a.new"), d.join("a.dat")),
            (d.join("b.new"), d.join("b.dat")),
        ])
        .unwrap();
        assert_eq!(std::fs::read_to_string(d.join("a.dat")).unwrap(), "new-a");
        assert_eq!(std::fs::read_to_string(d.join("b.dat")).unwrap(), "new-b");
        assert!(!d.join("a.dat.backup").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn replace_all_rolls_back_when_a_later_step_fails() {
        let d = scratch("rollback");
        std::fs::write(d.join("a.dat"), "old-a").unwrap();
        std::fs::write(d.join("a.new"), "new-a").unwrap();
        // 第二个新文件不存在 → rename 失败，必须回滚第一个
        let r = replace_all(&[
            (d.join("a.new"), d.join("a.dat")),
            (d.join("missing.new"), d.join("b.dat")),
        ]);
        assert!(r.is_err());
        assert_eq!(std::fs::read_to_string(d.join("a.dat")).unwrap(), "old-a");
        assert!(!d.join("b.dat").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn sha256_of_file_matches_known_vector() {
        let d = scratch("sha");
        std::fs::write(d.join("x"), b"abc").unwrap();
        assert_eq!(
            sha256_of_file(&d.join("x")).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert!(sha256_of_file(&d.join("nope")).is_none());
        let _ = std::fs::remove_dir_all(&d);
    }
}
