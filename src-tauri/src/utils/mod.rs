use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub fn hash_path(path: &str, mtime: i64) -> String {
    let mut hasher = Sha256::new();
    hasher.update(path.as_bytes());
    hasher.update(mtime.to_be_bytes());
    hex::encode(hasher.finalize())
}

pub fn thumbnail_path(cache_dir: &Path, path: &str, mtime: i64) -> PathBuf {
    let h = hash_path(path, mtime);
    cache_dir.join(format!("{}.jpg", &h[..16]))
}

const THUMB_MAX_SIDE: u32 = 360; // 180*2 for DPR (LIMITS.md §5.5)

/// Generate thumbnail (360px JPEG) via the `image` crate.
///
/// Note: a jpeg-decoder DCT-scaled fast path was tried and removed — it was
/// measured 44x SLOWER than full decode + thumbnail (12s vs 0.27s for a 24MP
/// JPEG, because the scaled path still entropy-decodes everything through a
/// slow serial IDCT). Plain image::open + thumbnail is the fast path.
pub fn generate_thumbnail(src: &Path, dst: &Path) -> Result<(), String> {
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let img = image::open(src).map_err(|e| e.to_string())?;
    let thumb = img.thumbnail(THUMB_MAX_SIDE, THUMB_MAX_SIDE);
    thumb.save(dst).map_err(|e| e.to_string())?;
    Ok(())
}

/// Cache size in bytes
pub fn dir_size(path: &Path) -> u64 {
    let mut total = 0u64;
    if let Ok(entries) = std::fs::read_dir(path) {
        for e in entries.flatten() {
            if let Ok(meta) = e.metadata() {
                if meta.is_file() {
                    total += meta.len();
                } else if meta.is_dir() {
                    total += dir_size(&e.path());
                }
            }
        }
    }
    total
}

/// Enforce 500 MB limit, evict oldest by mtime.
/// Throttled: this is called on every thumbnail generation, so at most one
/// full cache walk per 60s (CAS-guarded, safe under concurrent calls).
pub fn enforce_cache_limit(cache_dir: &Path, max_bytes: u64) {
    static LAST_CHECK: AtomicU64 = AtomicU64::new(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let last = LAST_CHECK.load(Ordering::SeqCst);
    if now < last + 60 {
        return;
    }
    if LAST_CHECK
        .compare_exchange(last, now, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return; // another call already took this window
    }

    let size = dir_size(cache_dir);
    if size <= max_bytes {
        return;
    }
    // Collect files with mtime
    let mut files: Vec<(PathBuf, u64, std::time::SystemTime)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(cache_dir) {
        for e in entries.flatten() {
            let p = e.path();
            if let Ok(meta) = std::fs::metadata(&p) {
                if meta.is_file() {
                    let mtime = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
                    files.push((p, meta.len(), mtime));
                }
            }
        }
    }
    files.sort_by_key(|(_, _, t)| *t);
    let mut cur = size;
    for (p, sz, _) in files {
        if cur <= max_bytes {
            break;
        }
        if std::fs::remove_file(&p).is_ok() {
            cur = cur.saturating_sub(sz);
            log::info!("evicted thumbnail {:?}", p);
        }
    }
}

pub fn cache_dir(app_dir: &Path) -> PathBuf {
    app_dir.join("cache").join("thumbnails")
}
