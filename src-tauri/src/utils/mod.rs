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

/// Unique temp suffix so concurrent writers (prewarm worker + frontend
/// get_thumbnail) never collide on the same temp file.
static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// Generate thumbnail (360px JPEG) via the `image` crate.
///
/// Note: a jpeg-decoder DCT-scaled fast path was tried and removed — it was
/// measured 44x SLOWER than full decode + thumbnail (12s vs 0.27s for a 24MP
/// JPEG, because the scaled path still entropy-decodes everything through a
/// slow serial IDCT). Plain image::open + thumbnail is the fast path.
///
/// Writes to a unique temp file then renames atomically: a concurrent
/// clear_cache (remove_dir_all) can never observe a half-written thumbnail,
/// and readers never see a partial file.
pub fn generate_thumbnail(src: &Path, dst: &Path) -> Result<(), String> {
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let img = decode_image(src)?;
    let thumb = img.thumbnail(THUMB_MAX_SIDE, THUMB_MAX_SIDE);
    let seq = TMP_SEQ.fetch_add(1, Ordering::SeqCst);
    let tmp = dst.with_extension(format!("tmp{}_{}", std::process::id(), seq));
    let _ = std::fs::remove_file(&tmp);
    // save_with_format: the temp name has no image extension, so the `image`
    // crate must not sniff the format from it (that silently fails).
    thumb
        .save_with_format(&tmp, image::ImageFormat::Jpeg)
        .map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, dst).map_err(|e| e.to_string())?;
    Ok(())
}

/// Camera RAW extensions we index (C-19.21). `image` can't decode these
/// directly — decode_image() falls back to the embedded JPEG preview.
pub const RAW_EXTS: &[&str] = &[
    "nef", "nrw", // Nikon
    "pef", "ptx", // Pentax
    "arw", "srf", "sr2", // Sony
    "crw", "cr2", "cr3", // Canon
    "dng", // Adobe Digital Negative
    "raf", // Fujifilm
    "orf", // Olympus
    "rw2", "raw", // Panasonic / Leica
    "srw", // Samsung
];

pub fn is_raw_ext(ext: &str) -> bool {
    RAW_EXTS.contains(&ext.to_ascii_lowercase().as_str())
}

pub fn is_raw_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(is_raw_ext)
        .unwrap_or(false)
}

/// Find the largest decodable JPEG embedded in a RAW file (C-19.21). Every
/// TIFF-based RAW (NEF/ARW/CR2/DNG/PEF/RAF/...) carries full-size JPEG
/// previews; instead of parsing each container's IFDs, scan the bytes for
/// JPEG SOI (FF D8 FF) markers, cut each segment at the next EOI (FF D9)
/// and decode candidates until one succeeds — the largest wins. Container
/// details differ per brand; this works across all of them. False SOIs
/// inside compressed data just fail to decode and are skipped.
fn extract_raw_preview(data: &[u8]) -> Option<image::DynamicImage> {
    let mut best: Option<(usize, image::DynamicImage)> = None; // (area, img)
    let mut i = 0;
    while i + 3 <= data.len() {
        if data[i] == 0xFF && data[i + 1] == 0xD8 && data[i + 2] == 0xFF {
            // Find the segment end: first EOI (FF D9) after the SOI.
            let mut end = None;
            let mut j = i + 3;
            while j + 1 < data.len() {
                if data[j] == 0xFF && data[j + 1] == 0xD9 {
                    end = Some(j + 2);
                    break;
                }
                j += 1;
            }
            let Some(end) = end else { break };
            // image 0.24.x's JPEG decoder PANICS (copy_from_slice length
            // mismatch) on some odd preview segments instead of returning
            // Err (C-19.21). catch_unwind keeps the worker alive; silence
            // the default panic hook during the call so each panicking
            // candidate does not spam the dev log — the unwind result is
            // still observed normally below.
            let prev_hook = std::panic::take_hook();
            std::panic::set_hook(Box::new(|_| {}));
            let dec = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                image::load_from_memory(&data[i..end])
            }));
            std::panic::set_hook(prev_hook);
            if let Ok(Ok(img)) = dec {
                let area = img.width() as usize * img.height() as usize;
                if best.as_ref().map_or(true, |(a, _)| area > *a) {
                    best = Some((area, img));
                }
            }
            i = end;
        } else {
            i += 1;
        }
    }
    best.map(|(_, img)| img)
}

/// Decode any supported image: straight `image::open` for normal formats,
/// embedded-JPEG extraction for camera RAW (C-19.21). Shared by thumbnail
/// generation AND the AI vision encoder so RAW files get thumbnails,
/// embeddings and semantic search from their preview image.
pub fn decode_image(path: &Path) -> Result<image::DynamicImage, String> {
    match image::open(path) {
        Ok(img) => Ok(img),
        Err(direct) if is_raw_path(path) => {
            let data = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
            // Guard: `image` errors like "unsupported format" can also come
            // from truncated files; the scan simply finds nothing then.
            extract_raw_preview(&data)
                .ok_or_else(|| format!("decode {}: {direct} (no embedded preview)", path.display()))
        }
        Err(e) => Err(e.to_string()),
    }
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

/// ADD.md §9.1: storage key for paths — absolute, UNC-safe (dunce::simplified),
/// forward slashes, lowercased. Used for DB uniqueness and cross-platform
/// consistency. The real (display) path is kept in `display_path` for IO/UI.
pub fn normalize_storage_path(p: &str) -> String {
    let path = Path::new(p);
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    let mut s = dunce::simplified(&abs)
        .to_string_lossy()
        .replace('\\', "/")
        .to_lowercase();
    while s.ends_with('/') {
        s.pop();
    }
    s
}
