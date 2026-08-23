use crate::db::Db;
use std::collections::HashSet;
use std::path::Path;
use walkdir::WalkDir;

const ALLOWED_EXTS: &[&str] = &[
    "jpg", "jpeg", "png", "webp", "gif", "bmp", "tiff", "tif", "heic", "heif", "mp4", "mov", "avi",
    "mkv",
];

fn is_allowed(path: &Path) -> bool {
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        let lower = ext.to_ascii_lowercase();
        ALLOWED_EXTS.contains(&lower.as_str())
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

/// Incremental scan for a single folder.
/// Returns (added_or_updated, deleted)
pub fn scan_folder(db: &Db, folder_id: i64, folder_path: &str) -> Result<(usize, usize), String> {
    let root = Path::new(folder_path);
    if !root.exists() {
        log::warn!("folder not exists: {}", folder_path);
        return Ok((0, 0));
    }

    let existing_map = db.get_file_map(folder_id)?;
    let mut seen: HashSet<String> = HashSet::new();
    let mut changed = 0usize;

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
        let abs = p.to_string_lossy().to_string();
        // Normalize to forward slashes for consistency
        let abs_norm = abs.replace('\\', "/");
        seen.insert(abs_norm.clone());

        let Some((size, mtime)) = file_mtime_size(p) else {
            continue;
        };
        let needs_update = match existing_map.get(&abs_norm) {
            Some((old_size, old_mtime)) => *old_size != size || *old_mtime != mtime,
            None => true,
        };
        if needs_update {
            let filename = p
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();
            db.upsert_file(folder_id, &abs_norm, &filename, size, mtime)?;
            changed += 1;
        }
    }

    let deleted = db.delete_missing(folder_id, &seen)?;
    if changed > 0 || deleted > 0 {
        log::info!(
            "scan folder {} -> changed: {}, deleted: {}",
            folder_path,
            changed,
            deleted
        );
    }
    Ok((changed, deleted))
}

/// Scan all folders with concurrency limit 2 (tokio semaphore style but sync version for now).
pub fn scan_all(db: &Db) -> Result<Vec<(i64, usize, usize)>, String> {
    let folders = db.get_folders()?;
    let mut results = Vec::new();
    // Simple sequential with limit 2 would be same as parallel 2 in sync context.
    // Keep sequential for simplicity; tokio wrapper will enforce concurrency.
    for f in folders {
        let r = scan_folder(db, f.id, &f.path)?;
        results.push((f.id, r.0, r.1));
    }
    Ok(results)
}
