//! Self-update detection (C-18): compare the running executable's SHA256
//! against https://tiol.netlify.app/version.json (published by CI on every
//! tag build — build.yml push-to-site-repo job). The version NUMBER is
//! display-only: the update decision is made purely by hash comparison, so
//! nothing has to be baked into the exe.
//!
//! Never fails loudly: offline, unreachable Netlify, malformed JSON or a
//! dev build all resolve to "no update" (the banner only appears when a
//! remote hash differs from ours).

use serde::Serialize;
use std::path::Path;

#[derive(Serialize)]
pub struct UpdateInfo {
    pub available: bool,
    pub version: Option<String>,
    pub url: Option<String>,
    pub local_sha: String,
    pub remote_sha: Option<String>,
}

/// SHA256 hex (lowercase) of a file, streamed in 1 MiB chunks (release exes
/// are tens of MB; streaming avoids loading them fully into memory).
pub fn sha256_hex(path: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 1024 * 1024];
    loop {
        let n = file.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Evaluate a fetched version.json against the local exe hash. Pure logic,
/// unit-testable. `platform` is "windows" | "macos".
pub fn evaluate_update(remote_json: &str, local_sha: &str, platform: &str) -> UpdateInfo {
    let local_sha = local_sha.trim().to_lowercase();
    let Ok(v) = serde_json::from_str::<serde_json::Value>(remote_json) else {
        return UpdateInfo {
            available: false,
            version: None,
            url: None,
            local_sha,
            remote_sha: None,
        };
    };
    let entry = v.get("platforms").and_then(|p| p.get(platform));
    let Some(remote_sha) = entry
        .and_then(|e| e.get("sha256"))
        .and_then(|s| s.as_str())
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
    else {
        return UpdateInfo {
            available: false,
            version: None,
            url: None,
            local_sha,
            remote_sha: None,
        };
    };
    let version = v.get("version").and_then(|s| s.as_str()).map(|s| s.to_string());
    let url = entry
        .and_then(|e| e.get("url"))
        .and_then(|s| s.as_str())
        .map(|s| s.to_string());
    UpdateInfo {
        available: remote_sha != local_sha,
        version,
        url,
        local_sha,
        remote_sha: Some(remote_sha),
    }
}

/// The update-check command. Every failure mode resolves to "no update".
pub async fn check_update() -> UpdateInfo {
    // Dev builds run from target/debug — their hash can never match a
    // release artifact, which would report "update available" forever.
    if cfg!(debug_assertions) {
        return no_update();
    }
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            log::debug!("update: current_exe failed: {e}");
            return no_update();
        }
    };
    let local_sha = match sha256_hex(&exe) {
        Ok(s) => s,
        Err(e) => {
            log::debug!("update: sha256 of {} failed: {e}", exe.display());
            return no_update();
        }
    };
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(_) => return no_update(),
    };
    let body = match client
        .get("https://tiol.netlify.app/version.json")
        .send()
        .await
    {
        Ok(r) => match r.text().await {
            Ok(t) => t,
            Err(_) => return no_update(),
        },
        Err(e) => {
            log::debug!("update: fetch version.json failed: {e}");
            return no_update();
        }
    };
    let platform = if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    };
    let info = evaluate_update(&body, &local_sha, platform);
    if info.available {
        log::info!(
            "update available: v{} (local {} != remote {})",
            info.version.as_deref().unwrap_or("?"),
            info.local_sha,
            info.remote_sha.as_deref().unwrap_or("?")
        );
    }
    info
}

fn no_update() -> UpdateInfo {
    UpdateInfo {
        available: false,
        version: None,
        url: None,
        local_sha: String::new(),
        remote_sha: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_hex_known_vector() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("f.txt");
        std::fs::write(&p, b"hello").unwrap();
        assert_eq!(
            sha256_hex(&p).unwrap(),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn evaluate_matching_sha_is_no_update() {
        let json = r#"{"version":"0.1.7","platforms":{"windows":{"sha256":"ABCD","url":"https://x/z.zip"}}}"#;
        let info = evaluate_update(json, "abcd", "windows");
        assert!(!info.available);
        assert_eq!(info.version.as_deref(), Some("0.1.7"));
        assert_eq!(info.url.as_deref(), Some("https://x/z.zip"));
        assert_eq!(info.remote_sha.as_deref(), Some("abcd"));
    }

    #[test]
    fn evaluate_mismatch_is_update() {
        let json = r#"{"version":"0.1.7","platforms":{"windows":{"sha256":"aaaa","url":"https://x/z.zip"}}}"#;
        let info = evaluate_update(json, "bbbb", "windows");
        assert!(info.available);
        assert_eq!(info.version.as_deref(), Some("0.1.7"));
        assert_eq!(info.url.as_deref(), Some("https://x/z.zip"));
    }

    #[test]
    fn evaluate_missing_platform_or_bad_json_is_no_update() {
        // Other platform's data must never trigger an update for us.
        let json = r#"{"version":"0.1.7","platforms":{"macos":{"sha256":"aaaa"}}}"#;
        let info = evaluate_update(json, "bbbb", "windows");
        assert!(!info.available);
        assert!(info.remote_sha.is_none());
        // Malformed JSON.
        let info2 = evaluate_update("not json", "bbbb", "windows");
        assert!(!info2.available);
        // Empty sha256 counts as missing.
        let json3 = r#"{"version":"0.1.7","platforms":{"windows":{"sha256":"","url":"u"}}}"#;
        let info3 = evaluate_update(json3, "bbbb", "windows");
        assert!(!info3.available);
    }
}
