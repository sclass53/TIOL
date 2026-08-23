#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod ai;
mod db;
mod scanner;
mod utils;
mod watcher;

use db::{Db, FileRecord, Folder};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::webview::PageLoadEvent;
use tauri::{Emitter, Manager};

/// Background thumbnail prewarm: after a scan, generate missing thumbnails
/// with a small worker pool so browsing is instant even for large libraries.
/// On-demand generation (get_thumbnail) still wins for the visible viewport.
fn spawn_thumb_prewarm(app_dir: std::path::PathBuf, db: Arc<Db>) {
    std::thread::spawn(move || {
        let cache = utils::cache_dir(&app_dir);
        let folders = match db.get_folders() {
            Ok(f) => f,
            Err(e) => {
                log::warn!("thumb prewarm: get_folders failed: {}", e);
                return;
            }
        };
        let mut paths: Vec<String> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for f in &folders {
            if let Ok(map) = db.get_file_map(f.id) {
                for p in map.keys() {
                    if seen.insert(p.clone()) {
                        paths.push(p.clone());
                    }
                }
            }
        }
        if paths.is_empty() {
            return;
        }
        log::info!("thumb prewarm: {} files", paths.len());
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        // 2 workers: gentle background load, keeps CPU/disk for on-demand
        // generation of whatever the user is actually looking at.
        let rx = Arc::new(std::sync::Mutex::new(rx));
        for _ in 0..2 {
            let rx = rx.clone();
            let cache = cache.clone();
            std::thread::spawn(move || {
                loop {
                    let path = {
                        let guard = rx.lock().unwrap();
                        guard.recv()
                    };
                    let path = match path {
                        Ok(p) => p,
                        Err(_) => break, // channel closed
                    };
                    let p = std::path::Path::new(&path);
                    let meta = match std::fs::metadata(p) {
                        Ok(m) => m,
                        Err(_) => continue,
                    };
                    let mtime = meta
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs() as i64)
                        .unwrap_or(0);
                    let thumb_path = utils::thumbnail_path(&cache, &path, mtime);
                    if thumb_path.exists() {
                        continue;
                    }
                    if let Err(e) = utils::generate_thumbnail(p, &thumb_path) {
                        log::debug!("prewarm thumb failed for {}: {}", path, e);
                    }
                }
            });
        }
        let mut queued = 0usize;
        for p in paths {
            if tx.send(p).is_err() {
                break;
            }
            queued += 1;
            if queued % 100 == 0 {
                log::info!("thumb prewarm: {queued} queued");
            }
        }
        drop(tx);
        log::info!("thumb prewarm: finished queueing {queued} files");
    });
}

struct AppState {
    db: Arc<Db>,
    ai: ai::MockSkill,
    app_dir: std::path::PathBuf,
}

#[tauri::command]
async fn add_folder(state: tauri::State<'_, AppState>, path: String) -> Result<i64, String> {
    log::info!("add_folder {}", path);
    let p = std::path::Path::new(&path);
    if !p.exists() || !p.is_dir() {
        return Err("Path does not exist or not a directory".to_string());
    }
    let id = state.db.add_folder(&path)?;
    // Scan before returning (off the main thread) so the folder's photo count
    // and the photo grid are final — no "numbers change after refresh" races.
    let db = state.db.clone();
    let path_clone = path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(e) = scanner::scan_folder(&db, id, &path_clone) {
            log::error!("initial scan failed: {}", e);
        }
    })
    .await
    .map_err(|e| e.to_string())?;
    spawn_thumb_prewarm(state.app_dir.clone(), state.db.clone());
    Ok(id)
}

#[tauri::command]
fn remove_folder(state: tauri::State<AppState>, id: i64) -> Result<(), String> {
    log::info!("remove_folder {}", id);
    state.db.remove_folder(id)
}

#[tauri::command]
fn get_folders(state: tauri::State<AppState>) -> Result<Vec<Folder>, String> {
    state.db.get_folders()
}

#[tauri::command]
fn get_photos(
    state: tauri::State<AppState>,
    folder_id: Option<i64>,
) -> Result<Vec<FileRecord>, String> {
    state.db.get_photos(folder_id)
}

#[tauri::command]
fn search_files(state: tauri::State<AppState>, query: String) -> Result<Vec<FileRecord>, String> {
    log::info!("search_files query={}", query);
    state.ai.search(&state.db, &query)
}

#[tauri::command]
fn search_description(state: tauri::State<AppState>, query: String) -> Result<Vec<FileRecord>, String> {
    log::info!("search_description query={}", query);
    state.db.search_description(&query)
}

#[tauri::command]
fn update_description(
    state: tauri::State<AppState>,
    id: i64,
    description: String,
) -> Result<(), String> {
    log::info!("update_description id={} desc={}", id, description);
    state.db.update_description(id, &description)
}

#[tauri::command]
fn get_setting(
    state: tauri::State<AppState>,
    key: String,
) -> Result<Option<String>, String> {
    state.db.get_setting(&key)
}

#[tauri::command]
fn set_setting(state: tauri::State<AppState>, key: String, value: String) -> Result<(), String> {
    log::info!("set_setting {}={}", key, value);
    state.db.set_setting(&key, &value)
}

#[tauri::command]
fn restart_app(app_handle: tauri::AppHandle) {
    log::info!("restart_app");
    app_handle.restart();
}

#[tauri::command]
fn report_renderer(renderer: String) {
    log::info!("webview renderer: {}", renderer);
}

#[tauri::command]
fn reveal_in_folder(path: String) -> Result<(), String> {
    log::info!("reveal_in_folder {}", path);
    #[cfg(target_os = "windows")]
    {
        // The DB stores forward-slash paths, but Explorer needs backslashes,
        // and "/select,<path>" must be ONE argument or it ignores the command.
        let win_path = path.replace('/', "\\");
        let arg = format!("/select,{}", win_path);
        std::process::Command::new("explorer")
            .arg(&arg)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("-R")
            .arg(&path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(parent) = std::path::Path::new(&path).parent() {
            std::process::Command::new("xdg-open")
                .arg(parent)
                .spawn()
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[tauri::command]
fn scan_folders(
    state: tauri::State<AppState>,
    app_handle: tauri::AppHandle,
) -> Result<Vec<(i64, usize, usize)>, String> {
    let folders = state.db.get_folders()?;
    let mut results = Vec::new();
    // Limit concurrency to 2 via simple chunking
    for chunk in folders.chunks(2) {
        for f in chunk {
            let (changed, deleted) = scanner::scan_folder(&state.db, f.id, &f.path)?;
            results.push((f.id, changed, deleted));
        }
        let _ = app_handle.emit("scan-progress", &results);
    }
    let _ = app_handle.emit("scan-complete", &results);
    spawn_thumb_prewarm(state.app_dir.clone(), state.db.clone());
    Ok(results)
}

#[tauri::command]
async fn get_thumbnail(
    state: tauri::State<'_, AppState>,
    path: String,
) -> Result<String, String> {
    // Thumbnail generation decodes the full source image (LIMITS.md §5.5);
    // run it off the main thread so the UI never blocks on it.
    let app_dir = state.app_dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        // Return thumbnail file path (generate if needed). Frontend uses convertFileSrc.
        let start = std::time::Instant::now();
        let p = std::path::Path::new(&path);
        let meta = match std::fs::metadata(p) {
            Ok(m) => m,
            Err(e) => {
                log::warn!("thumb metadata failed for {}: {}", path, e);
                // Empty string = "show placeholder", never load the original.
                return Ok(String::new());
            }
        };
        let mtime = meta
            .modified()
            .map_err(|e| e.to_string())?
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_secs() as i64;
        let cache = utils::cache_dir(&app_dir);
        let thumb_path = utils::thumbnail_path(&cache, &path, mtime);
        if !thumb_path.exists() {
            // Try generate; on failure (corrupt/unsupported/video) return an
            // empty path so the UI shows a placeholder INSTEAD of loading the
            // original file (decoding a corrupt full-size image stalls WebView).
            if let Err(e) = utils::generate_thumbnail(p, &thumb_path) {
                log::warn!("thumb gen failed for {}: {}", path, e);
                return Ok(String::new());
            }
            let elapsed = start.elapsed().as_millis();
            if elapsed > 10000 {
                log::warn!("thumb gen slow: {}ms for {}", elapsed, path);
            }
            // enforce 500MB limit async
            let c = cache.clone();
            std::thread::spawn(move || {
                utils::enforce_cache_limit(&c, 500 * 1024 * 1024);
            });
        }
        Ok(thumb_path.to_string_lossy().to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// One-time migration of app data from the old identifier
/// (com.imagemanager.demo) to the current one. Runs before the DB opens.
fn migrate_old_data(new_dir: &std::path::Path) {
    const OLD_IDENTIFIER: &str = "com.imagemanager.demo";
    let old_dir = match new_dir.parent() {
        Some(p) => p.join(OLD_IDENTIFIER),
        None => return,
    };
    if old_dir == new_dir {
        return;
    }
    let old_db = old_dir.join("db.sqlite");
    let new_db = new_dir.join("db.sqlite");
    if !old_db.exists() || new_db.exists() {
        return;
    }
    log::info!("migrating app data from {:?} to {:?}", old_dir, new_dir);
    if std::fs::create_dir_all(new_dir).is_err() {
        return;
    }
    for name in ["db.sqlite", "db.sqlite-wal", "db.sqlite-shm"] {
        let src = old_dir.join(name);
        if src.exists() {
            let _ = std::fs::copy(&src, new_dir.join(name));
        }
    }
    let old_cache = old_dir.join("cache");
    if old_cache.exists() {
        let _ = copy_dir_all(&old_cache, &new_dir.join("cache"));
    }
}

fn copy_dir_all(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &to)?;
        } else {
            std::fs::copy(entry.path(), &to)?;
        }
    }
    Ok(())
}

#[tauri::command]
async fn clear_cache(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let cache = utils::cache_dir(&state.app_dir);
    tauri::async_runtime::spawn_blocking(move || {
        log::info!("clearing thumbnail cache {:?}", cache);
        if cache.exists() {
            std::fs::remove_dir_all(&cache).map_err(|e| e.to_string())?;
        }
        Ok::<(), String>(())
    })
    .await
    .map_err(|e| e.to_string())?
}

fn main() {
    env_logger::init();
    // Use current_thread runtime per LIMITS.md:126 — Tauri will create its own runtime, we just limit our tasks.
    tauri::Builder::default()
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            let app_dir = app
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("."));
            // One-time migration from the old demo identifier
            // (com.imagemanager.demo) to the new one: move the database and
            // thumbnail cache so no data is lost after the rename.
            migrate_old_data(&app_dir);
            std::fs::create_dir_all(&app_dir).ok();
            // ensure cache dir
            std::fs::create_dir_all(utils::cache_dir(&app_dir)).ok();
            let db_path = app_dir.join("db.sqlite");
            log::info!("db path {:?}", db_path);
            let db = Arc::new(Db::new(&db_path).expect("failed to init db"));

            // Hardware-decoding setting: browser args only apply when the
            // WebView2 environment is created, so the config window has
            // "create": false and we build it here with the right args.
            let hw_args = match db.get_setting("hw_decode").ok().flatten().as_deref() {
                Some("1") => "--enable-gpu --ignore-gpu-blocklist --enable-accelerated-video-decode --enable-zero-copy",
                Some("0") => "--disable-gpu",
                _ => "",
            };
            log::info!("hw_decode browser args: {:?}", hw_args);
            if let Some(window_cfg) = app.config().app.windows.iter().find(|w| !w.create) {
                // Friendly in-app page replaces WebView2's built-in browser error
                // page (ERR_CONNECTION_REFUSED etc.) when a navigation fails.
                let scheme = if window_cfg.use_https_scheme {
                    "https"
                } else {
                    "http"
                };
                let error_url: url::Url =
                    format!("{scheme}://tauri.localhost/error.html")
                        .parse()
                        .expect("invalid error page url");
                let redirected = Arc::new(AtomicBool::new(false));
                let redirected_hook = redirected.clone();
                tauri::webview::WebviewWindowBuilder::from_config(app.handle(), window_cfg)?
                    .additional_browser_args(hw_args)
                    .on_page_load(move |window, payload| {
                        if payload.event() != PageLoadEvent::Finished {
                            return;
                        }
                        let url = payload.url().as_str();
                        // A finished navigation to anything that is not our own
                        // page means the load failed and WebView2 is showing its
                        // built-in error page — replace it (once) with ours.
                        let is_app_page = url.ends_with("index.html")
                            || url.ends_with("error.html")
                            || url.ends_with('/');
                        if !is_app_page && !redirected_hook.swap(true, Ordering::SeqCst) {
                            log::warn!(
                                "navigation failed or unexpected page ({}), showing in-app error page",
                                url
                            );
                            let _ = window.navigate(error_url.clone());
                        }
                    })
                    .build()?;
            }
            // startup incremental scan (LIMITS.md:107); emit scan-complete so
            // the UI refreshes with final counts/photo list when it finishes,
            // then prewarm thumbnails in the background.
            let db_clone = db.clone();
            let handle = app.handle().clone();
            let prewarm_dir = app_dir.clone();
            std::thread::spawn(move || {
                if let Ok(folders) = db_clone.get_folders() {
                    for f in folders {
                        if let Err(e) = scanner::scan_folder(&db_clone, f.id, &f.path) {
                            log::error!("startup scan {} failed: {}", f.path, e);
                        }
                    }
                }
                let _ = handle.emit("scan-complete", Vec::<(i64, usize, usize)>::new());
                spawn_thumb_prewarm(prewarm_dir, db_clone);
            });
            // file watcher — needs tokio runtime, spawn with tokio if available
            // For MVP we skip watcher in setup to avoid tokio runtime missing; watcher is started lazily on demand
            // Store state
            app.manage(AppState {
                db,
                ai: ai::MockSkill::new(),
                app_dir,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            add_folder,
            remove_folder,
            get_folders,
            get_photos,
            search_files,
            search_description,
            update_description,
            get_setting,
            set_setting,
            restart_app,
            report_renderer,
            reveal_in_folder,
            clear_cache,
            scan_folders,
            get_thumbnail
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
