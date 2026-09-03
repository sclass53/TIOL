use crate::db::Db;
use crate::utils::normalize_storage_path;
use std::collections::HashSet;
use std::path::Path;
use walkdir::WalkDir;

const ALLOWED_EXTS: &[&str] = &[
    "jpg", "jpeg", "png", "webp", "gif", "bmp", "tiff", "tif", "heic", "heif",
    // Camera RAW (C-19.21): NEF/ARW/CR2/... — decoded via the embedded JPEG
    // preview (utils::decode_image). Videos (.mp4/...) stay excluded (C-11.7).
];
fn is_allowed(path: &Path) -> bool {
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        let lower = ext.to_ascii_lowercase();
        ALLOWED_EXTS.contains(&lower.as_str()) || crate::utils::is_raw_ext(&lower)
    } else {
        false
    }
}

fn file_mtime_size(path: &Path) -> Option<(i64, i64)> {
    let meta = std::fs::metadata(path).ok()?;
    let size = meta.len() as i64;
    let mtime = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs() as i64;
    Some((size, mtime))
}

/// Incremental scan for a single folder (ADD.md §3.3).
/// Returns (added_or_updated, deleted, pending_ai_file_ids).
pub fn scan_folder(db: &Db, folder_id: i64, folder_path: &str) -> Result<(usize, usize, Vec<i64>), String> {
    let root = Path::new(folder_path);
    if !root.exists() {
        log::warn!("folder not exists: {}", folder_path);
        return Ok((0, 0, Vec::new()));
    }

    let existing_map = db.get_file_map(folder_id)?;
    let mut seen: HashSet<String> = HashSet::new();
    let mut changed = 0usize;
    let mut pending: Vec<i64> = Vec::new();

    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        if !is_allowed(p) {
            continue;
        }
        let display = p.to_string_lossy().to_string();
        // Storage key: dunce-simplified + forward slashes + lowercase (ADD.md §9.1).
        let key = normalize_storage_path(&display);
        seen.insert(key.clone());

        let Some((size, mtime)) = file_mtime_size(p) else {
            continue;
        };
        let needs_update = match existing_map.get(&key) {
            Some((old_size, old_mtime)) => *old_size != size || *old_mtime != mtime,
            None => true,
        };
        if needs_update {
            let filename = p
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();
            let id = db.upsert_file(folder_id, &key, &display, &filename, size, mtime)?;
            // NOTE: EXIF is NOT extracted here anymore (C-19.8) — reading
            // every file synchronously made folder scans visibly slow. New
            // files keep exif_checked=0 and a background backfill task picks
            // them up right after the scan completes.
            pending.push(id);
            changed += 1;
        }
    }

    let deleted = db.delete_missing(folder_id, &seen)?;
    db.update_last_scan_time(folder_id, chrono::Utc::now().timestamp())?;
    if changed > 0 || deleted > 0 {
        log::info!(
            "scan folder {} -> changed: {}, deleted: {}, ai_pending: {}",
            folder_path,
            changed,
            deleted,
            pending.len()
        );
    }
    Ok((changed, deleted, pending))
}


