//! Model downloader (ADD.md §4 + 补充章节): disk-space precheck, resumable
//! downloads (.part + Range), SHA256 verification, atomic rename, mirror
//! fallback (hf-mirror -> openi -> huggingface.co; modelscope excluded — its
//! path structure is not HF-compatible), progress events to the frontend.

use crate::ai::model_lock::{ModelFileInfo, ModelStatus, MODEL_LOCK};
use crate::error::{AppError, Result};
use futures_util::StreamExt;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter};
use tokio::io::AsyncWriteExt;

const MIRRORS: &[&str] = &[
    "https://hf-mirror.com",
    "https://openi.org.cn/hf-models",
    "https://huggingface.co",
];
const RESUME_ATTEMPTS: u32 = 3;

#[derive(Clone, Debug, Serialize)]
pub struct ModelDownloadEvent {
    pub status: String, // checking | downloading | verifying | locked | error
    pub file_name: String,
    pub progress: f32,
    pub message: String,
    pub sha256_mismatch: Option<String>,
    pub mirror: String,
}

/// Atomic verification of a single file against its lock entry.
pub fn verify_file(dir: &Path, file_name: &str, info: &ModelFileInfo) -> std::result::Result<(), String> {
    let file = dir.join(file_name);
    match std::fs::metadata(&file) {
        Ok(m) => {
            if m.len() != info.size {
                return Err(format!("size mismatch: expected {}, got {}", info.size, m.len()));
            }
        }
        Err(_) => return Err("missing".to_string()),
    }
    let mut hasher = Sha256::new();
    let bytes = std::fs::read(&file).map_err(|e| e.to_string())?;
    hasher.update(&bytes);
    let got = hex::encode(hasher.finalize());
    if !got.eq_ignore_ascii_case(info.sha256) {
        return Err(format!("expected {}, got {}", info.sha256, got));
    }
    Ok(())
}

fn sha256_of(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// Free bytes on the volume containing `path` (safety precheck).
pub fn free_space(path: &Path) -> u64 {
    use sysinfo::Disks;
    let disks = Disks::new_with_refreshed_list();
    let p = path.to_string_lossy().to_lowercase();
    let mut best: Option<u64> = None;
    for d in &disks {
        let mount = d.mount_point().to_string_lossy().to_lowercase();
        if p.starts_with(&mount) {
            let free = d.available_space();
            best = Some(match best {
                Some(b) => b.min(free),
                None => free,
            });
        }
    }
    best.unwrap_or(0)
}

fn emit_event(app: Option<&AppHandle>, status: &str, file: &str, progress: f32, message: &str, mismatch: Option<String>, mirror: &str) {
    if let Some(a) = app {
        let _ = a.emit(
            "model-download",
            ModelDownloadEvent {
                status: status.to_string(),
                file_name: file.to_string(),
                progress,
                message: message.to_string(),
                sha256_mismatch: mismatch,
                mirror: mirror.to_string(),
            },
        );
    }
}

/// Download a single file (resume + verify + atomic rename), trying mirrors
/// in order. `emit` sends progress events.
async fn download_one(
    client: &reqwest::Client,
    dir: &Path,
    name: &str,
    info: &ModelFileInfo,
    app: Option<&AppHandle>,
    progress_base: f32,
    progress_span: f32,
) -> Result<()> {
    let tmp = dir.join(format!("{name}.part"));
    let final_path = dir.join(name);

    // Disk space precheck: total + 500MB margin.
    let needed = info.size + 500 * 1024 * 1024;
    let free = free_space(dir);
    if free > 0 && free < needed {
        return Err(AppError::Download(format!(
            "not enough disk space: need ~{}MB, have {}MB",
            needed / 1024 / 1024,
            free / 1024 / 1024
        )));
    }

    let _ = std::fs::remove_file(&tmp);

    let mut last_error: Option<String> = None;
    for mirror in MIRRORS {
        let upstream = if info.url.starts_with("https://hf-mirror.com") {
            info.url.replace("https://hf-mirror.com", mirror)
        } else {
            info.url.to_string()
        };
        emit_event(app, "downloading", name, progress_base, &format!("downloading from {mirror}"), None, mirror);

        let mut attempt = 0u32;
        let mut verified = false;
        while attempt < RESUME_ATTEMPTS {
            let offset = std::fs::metadata(&tmp).map(|m| m.len()).unwrap_or(0);
            let mut req = client.get(&upstream);
            if offset > 0 {
                req = req.header(reqwest::header::RANGE, format!("bytes={offset}-"));
            }
            let resp = match req.send().await {
                Ok(r) => r,
                Err(e) => {
                    attempt += 1;
                    last_error = Some(e.to_string());
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    continue;
                }
            };
            let status = resp.status();
            if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
                let _ = std::fs::remove_file(&tmp);
                continue;
            }
            if !status.is_success() && status != reqwest::StatusCode::PARTIAL_CONTENT {
                last_error = Some(format!("HTTP {status}"));
                break; // next mirror
            }
            let mut file = tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&tmp)
                .await
                .map_err(|e| AppError::Download(e.to_string()))?;
            let mut stream = resp.bytes_stream();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|e| AppError::Download(e.to_string()))?;
                file.write_all(&chunk)
                    .await
                    .map_err(|e| AppError::Download(e.to_string()))?;
                let sz = file.metadata().await.map(|m| m.len()).unwrap_or(0);
                let p = progress_base + progress_span * (sz.min(info.size) as f32 / info.size.max(1) as f32);
                emit_event(app, "downloading", name, p, "", None, mirror);
            }
            file.flush().await.map_err(|e| AppError::Download(e.to_string()))?;
            drop(file);
            verified = true;
            break;
        }
        if !verified {
            continue; // attempts exhausted -> next mirror
        }

        // Verify + atomic rename.
        let bytes = match tokio::fs::read(&tmp).await {
            Ok(b) => b,
            Err(e) => {
                last_error = Some(e.to_string());
                continue;
            }
        };
        if bytes.len() as u64 != info.size {
            last_error = Some(format!("size mismatch: expected {}, got {}", info.size, bytes.len()));
            continue;
        }
        let got = sha256_of(&bytes);
        if !got.eq_ignore_ascii_case(info.sha256) {
            last_error = Some(format!("expected {}, got {}", info.sha256, got));
            emit_event(
                app,
                "verifying",
                name,
                progress_base + progress_span,
                "hash mismatch, switching mirror",
                Some(format!("Expected {}, Got {}", info.sha256, got)),
                mirror,
            );
            let _ = std::fs::remove_file(&tmp);
            continue; // next mirror
        }
        std::fs::rename(&tmp, &final_path).map_err(|e| AppError::Download(e.to_string()))?;
        emit_event(app, "locked", name, progress_base + progress_span, "verified", None, mirror);
        return Ok(());
    }
    let _ = std::fs::remove_file(&tmp);
    Err(AppError::Download(format!(
        "all mirrors failed for {name}: {}",
        last_error.unwrap_or_else(|| "unknown".to_string())
    )))
}

/// Check the model dir; download anything missing/corrupt. Returns the final
/// status. `app` is used for progress events (None in tests).
pub async fn ensure_models(model_dir: PathBuf, app: Option<AppHandle>) -> Result<ModelStatus> {
    use crate::ai::model_lock::ModelStatus;
    std::fs::create_dir_all(&model_dir).map_err(AppError::Io)?;

    let client = reqwest::Client::builder()
        .user_agent("tiol-model-downloader")
        .build()
        .map_err(|e| AppError::Download(e.to_string()))?;

    let total = MODEL_LOCK.len() as f32;
    for (i, (name, info)) in MODEL_LOCK.iter().enumerate() {
        let base = i as f32 / total;
        let span = 1.0 / total;
        match verify_file(&model_dir, name, info) {
            Ok(()) => emit_event(app.as_ref(), "locked", name, base + span, "verified (cached)", None, "-"),
            Err(reason) => {
                log::warn!("model {} invalid ({}), redownloading", name, reason);
                // poison file -> delete and re-download (ADD.md §4)
                let f = model_dir.join(name);
                let _ = std::fs::remove_file(&f);
                let _ = std::fs::remove_file(model_dir.join(format!("{name}.part")));
                download_one(&client, &model_dir, name, info, app.as_ref(), base, span).await?;
            }
        }
    }

    // Re-verify everything (ADD.md: all-or-nothing lock).
    for (name, info) in MODEL_LOCK {
        if let Err(reason) = verify_file(&model_dir, name, info) {
            return Err(AppError::Model(format!("{name}: {reason}")));
        }
    }
    emit_event(app.as_ref(), "locked", "-", 1.0, "all models verified", None, "-");
    Ok(ModelStatus::Locked("cpu".to_string())) // backend decided at engine load
}

pub static DOWNLOADING: AtomicBool = AtomicBool::new(false);

fn mark_downloading() -> bool {
    !DOWNLOADING.swap(true, Ordering::SeqCst)
}

fn mark_download_done() {
    DOWNLOADING.store(false, Ordering::SeqCst);
}

/// Background task: run ensure_models once. Degrades gracefully on failure.
pub async fn init_models_async(model_dir: PathBuf, app: AppHandle) -> ModelStatus {
    use crate::ai::model_lock::ModelStatus;
    if !mark_downloading() {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        return if MODEL_LOCK.iter().all(|(n, i)| verify_file(&model_dir, n, i).is_ok()) {
            ModelStatus::Locked("cpu".to_string())
        } else {
            ModelStatus::Degraded("models not ready".to_string())
        };
    }
    let result = ensure_models(model_dir.clone(), Some(app)).await;
    mark_download_done();
    match result {
        Ok(s) => s,
        Err(e) => {
            log::error!("model init failed: {}", e);
            ModelStatus::Degraded(e.to_string())
        }
    }
}

/// Map a ModelStatus to a user-facing event.
pub fn emit_status(app: &AppHandle, status: &ModelStatus) {
    use crate::ai::model_lock::ModelStatus;
    let (status_str, msg) = match status {
        ModelStatus::Locked(_) => ("locked", "all models verified".to_string()),
        ModelStatus::Degraded(r) => ("error", format!("AI degraded: {r}")),
    };
    emit_event(Some(app), status_str, "-", 1.0, &msg, None, "-");
}
