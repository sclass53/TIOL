#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod ai;
mod db;
mod error;
mod exif;
mod logbuf;
mod rejects;
mod scanner;
mod search;
mod update;
mod utils;
mod watcher;

use ai::engine::AIEngine;
use ai::model_lock::ModelStatus;
use db::{Db, FileRecord, Folder};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::io::Write as _;
use tauri::webview::PageLoadEvent;
use tauri::{Emitter, Manager};
use tokio::sync::Mutex as AsyncMutex;

/// Background thumbnail prewarm: after a scan, generate missing thumbnails
/// with a small worker pool so browsing is instant even for large libraries.
fn spawn_thumb_prewarm(app_dir: std::path::PathBuf, db: Arc<Db>) {    std::thread::spawn(move || {
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
        let rx = Arc::new(std::sync::Mutex::new(rx));
        for _ in 0..2 {
            let rx = rx.clone();
            let cache = cache.clone();
            std::thread::spawn(move || loop {
                let path = {
                    let guard = rx.lock().unwrap();
                    guard.recv()
                };
                let path = match path {
                    Ok(p) => p,
                    Err(_) => break,
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

/// Background EXIF backfill (C-15 / C-19.8): read lens/focal for every file
/// still marked exif_checked=0 (new/changed files since the last pass; files
/// without EXIF are marked checked too, so each file is read once). Runs in
/// a background thread — NEVER on the scan path, so adding a folder stays
/// fast. Called at startup and after every scan.
fn spawn_exif_backfill(db: Arc<Db>) {
    std::thread::spawn(move || {
        let mut offset = 0i64;
        loop {
            let batch = match db.get_files_missing_exif(500, offset) {
                Ok(b) => b,
                Err(e) => {
                    log::warn!("exif backfill query failed: {e}");
                    return;
                }
            };
            if batch.is_empty() {
                break;
            }
            for (id, path) in &batch {
                let (lens, focal) = exif::read_lens_focal(std::path::Path::new(path));
                if let Err(e) = db.update_exif(*id, lens.as_deref(), focal) {
                    log::warn!("exif backfill update failed for {path}: {e}");
                }
            }
            offset += batch.len() as i64;
        }
        log::info!("exif backfill complete");
    });
}

struct AppState {
    db: Arc<Db>,
    ai: ai::MockSkill,
    app_dir: std::path::PathBuf,
    model_dir: std::path::PathBuf,
    ai_queue: ai::queue::AITaskSender,
    ai_control: Arc<ai::queue::AIControl>,
    ai_engine: Arc<AsyncMutex<Option<AIEngine>>>,
    tag_cache: Arc<std::sync::RwLock<Vec<ai::engine::TagVec>>>,
    ai_status: Arc<std::sync::Mutex<Option<ModelStatus>>>,
    watcher: Arc<std::sync::Mutex<Option<watcher::FileWatcher>>>,
    /// Guards concurrent reject-metrics runs (startup warmup + entering the
    /// rejects page can both fire it; a duplicate run is a no-op).
    reject_analysis_running: Arc<std::sync::atomic::AtomicBool>,
}

/// Load (or reload) the AI engine with the provider mode from settings
/// ("auto" | "gpu" | "cpu"). Updates the shared status afterwards.
fn spawn_engine_load(
    model_dir: std::path::PathBuf,
    engine_holder: Arc<AsyncMutex<Option<AIEngine>>>,
    status_holder: Arc<std::sync::Mutex<Option<ModelStatus>>>,
    db: Arc<Db>,
) {
    tauri::async_runtime::spawn(async move {
        let mode = db
            .get_setting("ai_provider")
            .ok()
            .flatten()
            .unwrap_or_else(|| "auto".to_string());
        // ort can PANIC (e.g. `expect("Failed to load ONNX Runtime dylib")`
        // when the runtime library is missing) — panics only reach stderr,
        // invisible in a GUI app, so the watchdog would retry silently
        // forever. Catch it and surface the reason (C-11.12).
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            AIEngine::load(&model_dir, &mode)
        }));
        match result {
            Ok(Ok((engine, backend))) => {
                *engine_holder.lock().await = Some(engine);
                log::info!("AI engine loaded (backend={})", backend);
                *status_holder.lock().unwrap() = Some(ModelStatus::Locked(backend));
            }
            Ok(Err(e)) => {
                log::error!("AI engine load failed: {}", e);
                *status_holder.lock().unwrap() = Some(ModelStatus::Degraded(e.to_string()));
            }
            Err(panic) => {
                let msg = panic
                    .downcast_ref::<&str>()
                    .map(|s| s.to_string())
                    .or_else(|| panic.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "unknown panic".to_string());
                // The #1 macOS cause: the ONNX Runtime dynamic library is
                // missing next to the executable (see BUILD.md §2).
                let hint = format!(
                    "engine load panicked: {msg} — check that the ONNX Runtime library \
                     (libonnxruntime.dylib / onnxruntime.dll) sits next to the executable \
                     (BUILD.md §2)"
                );
                log::error!("{}", hint);
                *status_holder.lock().unwrap() = Some(ModelStatus::Degraded(hint));
            }
        }
    });
}

/// (Re)build the file watcher for the current folder list. Watcher scans
/// enqueue changed files into the AI queue and emit scan-complete.
fn rebuild_watcher(state: &AppState, app: tauri::AppHandle) {
    let db = state.db.clone();
    let keys = match db.get_folder_keys() {
        Ok(k) => k,
        Err(e) => {
            log::warn!("watcher: get_folder_keys failed: {}", e);
            return;
        }
    };
    if keys.is_empty() {
        return;
    }
    let mut guard = state.watcher.lock().unwrap();
    match watcher::FileWatcher::start(
        db.clone(),
        keys,
        state.ai_queue.clone(),
        state.ai_control.clone(),
        app,
    ) {
        Ok(w) => {
            *guard = Some(w);
            log::info!("file watcher started");
        }
        Err(e) => log::warn!("watcher start failed: {}", e),
    }
}

#[tauri::command]
async fn add_folder(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
    path: String,
) -> Result<i64, String> {
    log::info!("add_folder {}", path);
    let p = std::path::Path::new(&path);
    if !p.exists() || !p.is_dir() {
        return Err("Path does not exist or not a directory".to_string());
    }
    let id = state.db.add_folder(&path)?;
    let db = state.db.clone();
    let tx = state.ai_queue.clone();
    let epoch = state.ai_control.epoch();
    let path_clone = path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        match scanner::scan_folder(&db, id, &path_clone) {
            Ok((_, _, pending)) => {
                for fid in pending {
                    if let Ok(Some(rec)) = db.get_file_by_id(fid) {
                        let _ = tx.try_send(ai::queue::AITask::new(fid, rec.path, epoch));
                    }
                }
            }
            Err(e) => log::error!("initial scan failed: {}", e),
        }
    })
    .await
    .map_err(|e| e.to_string())?;
    spawn_thumb_prewarm(state.app_dir.clone(), state.db.clone());
    spawn_exif_backfill(state.db.clone());
    rebuild_watcher(&state, app);
    Ok(id)
}

#[tauri::command]
fn remove_folder(
    state: tauri::State<AppState>,
    app: tauri::AppHandle,
    id: i64,
) -> Result<(), String> {
    log::info!("remove_folder {}", id);
    state.db.remove_folder(id)?;
    // Invalidate queued AI work for the removed folder: tasks enqueued so far
    // carry an older epoch and are skipped by the consumer instead of
    // blocking newer work (ADD.md §5, user request).
    state.ai_control.invalidate();
    rebuild_watcher(&state, app);
    Ok(())
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

/// ADD.md §6/§7: dual-path search. mode = "tag" | "semantic".
#[tauri::command]
async fn search(
    state: tauri::State<'_, AppState>,
    query: String,
    mode: String,
) -> Result<Vec<FileRecord>, String> {
    log::info!("search query={} mode={}", query, mode);
    match mode.as_str() {
        "tag" => search::tag_search(&state.db, &query).map_err(|e| e.to_string()),
        "semantic" => {
            let guard = state.ai_engine.lock().await;
            match guard.as_ref() {
                Some(eng) => search::semantic_search(&state.db, eng, &query).map_err(|e| e.to_string()),
                None => Err("AI engine not ready (models not locked)".to_string()),
            }
        }
        other => Err(format!("unknown search mode: {other}")),
    }
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

/// Replace a file's MANUAL tags (source=0) with the given comma-separated
/// list. Returns the updated record (tags included) so the UI can refresh
/// the card without a full re-render.
#[tauri::command]
fn update_tags(
    state: tauri::State<AppState>,
    file_id: i64,
    tags: Vec<String>,
) -> Result<db::FileRecord, String> {
    log::info!("update_tags file_id={} tags={:?}", file_id, tags);
    state.db.replace_manual_tags(file_id, &tags)?;
    state
        .db
        .get_file_by_id(file_id)?
        .ok_or_else(|| "file not found".to_string())
}

/// All tags of one file (name, confidence, source) — the edit dialog uses
/// source=0 entries as the "manual tags" it edits.
#[tauri::command]
fn get_file_tags(state: tauri::State<AppState>, file_id: i64) -> Result<Vec<db::FileTag>, String> {
    state.db.get_file_tags(file_id)
}

/// Star rating (C-17): 1-5 stars, 0 clears. Returns the updated record so
/// the frontend can refresh the card in place without a full re-render.
#[tauri::command]
fn set_rating(
    state: tauri::State<AppState>,
    file_id: i64,
    rating: i64,
) -> Result<db::FileRecord, String> {
    if !(0..=5).contains(&rating) {
        return Err("rating must be between 0 and 5".to_string());
    }
    log::info!("set_rating file_id={} rating={}", file_id, rating);
    state.db.set_rating(file_id, rating)?;
    state
        .db
        .get_file_by_id(file_id)?
        .ok_or_else(|| "file not found".to_string())
}

/// Batch star rating (C-19): apply ONE rating to many files at once
/// (multi-select "Rate" button). 0 clears the rating on all of them.
#[tauri::command]
fn set_rating_files(
    state: tauri::State<AppState>,
    file_ids: Vec<i64>,
    rating: i64,
) -> Result<usize, String> {
    if !(0..=5).contains(&rating) {
        return Err("rating must be between 0 and 5".to_string());
    }
    if file_ids.is_empty() {
        return Err("no files selected".to_string());
    }
    log::info!("set_rating_files: {} files, rating={}", file_ids.len(), rating);
    for id in &file_ids {
        state.db.set_rating(*id, rating)?;
    }
    Ok(file_ids.len())
}

/// Reject-condition analysis (C-19.3). Two parts:
/// 1. Eyes-closed: cosine of every stored embedding vs "closed eyes"
///    text embedding, > 0.11 → eyes_closed=1. Recomputed for ALL files
///    with embeddings (fast: one text embed + a pass over ~3MB of vectors).
/// 2. Exposure (over/underexposed): pixel-luma statistics, INCREMENTAL —
///    only files whose overexposed is still NULL (new/changed files get
///    re-analyzed; unreadable images are marked 0/0 so they don't retry).
///    Runs in a blocking task and emits reject-analysis-progress /
///    reject-analysis-complete so the UI can show "analyzing…".
#[tauri::command]
async fn compute_reject_metrics(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    // Duplicate runs (startup warmup + page entry) are no-ops — the work is
    // incremental anyway, but one pass at a time keeps progress readable.
    if state
        .reject_analysis_running
        .swap(true, std::sync::atomic::Ordering::SeqCst)
    {
        return Ok(());
    }
    log::info!("compute_reject_metrics");
    // Everything below in one async block so the running flag is ALWAYS
    // released, even when the eyes-closed step errors out early.
    let result: Result<(), String> = async {
        // 1) Eyes-closed via stored embeddings (no image decode) — INCREMENTAL:
        //    only files whose eyes_closed is still NULL get checked (C-19.6).
        {
            let guard = state.ai_engine.lock().await;
            if let Some(eng) = guard.as_ref() {
                let q = eng.embed_text("closed eyes").map_err(|e| e.to_string())?;
                drop(guard);
                let rows = state.db.get_embeddings_missing_eyes().map_err(|e| e.to_string())?;
                let mut closed = 0usize;
                for (id, emb) in &rows {
                    let eyes = ai::engine::cosine(emb, &q) > 0.11;
                    state.db.update_reject_eyes(*id, eyes).map_err(|e| e.to_string())?;
                    if eyes {
                        closed += 1;
                    }
                }
                log::info!("reject metrics: eyes-closed checked {} files ({} closed)", rows.len(), closed);
            }
        }
        // 2) Exposure: incremental, thumbnail-first decode in a blocking task.
        let db = state.db.clone();
        let app_dir = state.app_dir.clone();
        let total = db.count_files_missing_exposure().map_err(|e| e.to_string())?;
        let handle = app.clone();
        tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
            let cache = utils::cache_dir(&app_dir);
            let mut offset = 0i64;
            let mut done = 0i64;
            let mut failed = 0usize;
            loop {
                let batch = db.get_files_missing_exposure(50, offset).map_err(|e| e.to_string())?;
                if batch.is_empty() {
                    break;
                }
                for (id, path) in &batch {
                    // Thumbnail pixels are enough for exposure stats and are
                    // far cheaper to decode than originals (C-19.4). A single
                    // bad file must NEVER abort the pass — catch panics and
                    // record errors, mark it as "neither" and continue so one
                    // run always completes the whole library (C-19.6).
                    let src = rejects::thumbnail_or_original(&cache, std::path::Path::new(path));
                    let verdict = std::panic::catch_unwind(|| {
                        rejects::analyze_exposure(&src)
                    });
                    let (over, under) = match verdict {
                        Ok(v) => v,
                        Err(_) => {
                            failed += 1;
                            (false, false)
                        }
                    };
                    if let Err(e) = db.update_reject_exposure(*id, over, under) {
                        log::warn!("reject exposure update failed for {path}: {e}");
                        failed += 1;
                    }
                    done += 1;
                    if done % 20 == 0 || done == total {
                        let _ = handle.emit(
                            "reject-analysis-progress",
                            serde_json::json!({ "done": done, "total": total }),
                        );
                    }
                }
                offset += batch.len() as i64;
            }
            let _ = handle.emit("reject-analysis-complete", serde_json::json!({}));
            log::info!(
                "reject metrics: exposure analyzed {done} files ({} failed, treated as normal)",
                failed
            );
            Ok(())
        })
        .await
        .map_err(|e| e.to_string())??;
        Ok(())
    }
    .await;
    state
        .reject_analysis_running
        .store(false, std::sync::atomic::Ordering::SeqCst);
    result
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
    let mut pending: Vec<i64> = Vec::new();
    for chunk in folders.chunks(2) {
        for f in chunk {
            let (changed, deleted, mut p) = scanner::scan_folder(&state.db, f.id, &f.path)?;
            results.push((f.id, changed, deleted));
            pending.append(&mut p);
        }
        let _ = app_handle.emit("scan-progress", &results);
    }
    let _ = app_handle.emit("scan-complete", &results);
    // Enqueue new/changed files for AI processing.
    let tx = state.ai_queue.clone();
    let db = state.db.clone();
    let epoch = state.ai_control.epoch();
    std::thread::spawn(move || {
        for fid in pending {
            if let Ok(Some(rec)) = db.get_file_by_id(fid) {
                let _ = tx.try_send(ai::queue::AITask::new(fid, rec.path, epoch));
            }
        }
    });
    spawn_thumb_prewarm(state.app_dir.clone(), state.db.clone());
    spawn_exif_backfill(state.db.clone());
    rebuild_watcher(&state, app_handle);
    Ok(results)
}

/// ADD.md §7: manually trigger AI processing for a file.
#[tauri::command]
fn process_file(state: tauri::State<AppState>, file_id: i64) -> Result<(), String> {
    let rec = state
        .db
        .get_file_by_id(file_id)?
        .ok_or_else(|| "file not found".to_string())?;
    state
        .ai_queue
        .try_send(ai::queue::AITask::new(file_id, rec.path, state.ai_control.epoch()))
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// ADD.md §7: add a few-shot custom tag from reference images (mean embedding).
/// Rebuild the in-memory tag vector cache from the DB (embeds every tag with
/// the SigLIP text encoder). Shared by the queue's tag matching.
async fn rebuild_tag_cache(
    db: Arc<Db>,
    engine: Arc<AsyncMutex<Option<AIEngine>>>,
    cache: Arc<std::sync::RwLock<Vec<ai::engine::TagVec>>>,
) {
    let tags = match db.get_custom_tags() {
        Ok(t) => t,
        Err(e) => {
            log::warn!("tag cache: get_custom_tags failed: {}", e);
            return;
        }
    };
    let mut out: Vec<ai::engine::TagVec> = Vec::new();
    {
        let guard = engine.lock().await;
        if let Some(eng) = guard.as_ref() {
            for t in &tags {
                match eng.embed_text(&t.name) {
                    Ok(v) => out.push(ai::engine::TagVec {
                        name: t.name.clone(),
                        threshold: t.threshold,
                        vec: v,
                    }),
                    Err(e) => log::warn!("tag cache: embed_text({}) failed: {}", t.name, e),
                }
            }
        }
    }
    if let Ok(mut g) = cache.write() {
        *g = out;
    }
    log::info!("tag cache: {} tags embedded", tags.len());
}

/// ADD.md §7 / MIGRATE1.md §2.2: add a USER-DEFINED text tag. Its text
/// embedding (SigLIP) is stored in custom_tags and cached in memory.
/// Adding a tag NEVER starts tagging (C-12): the manual "AI Tagging"
/// button (run_ai_tagging) matches every photo against ALL current tags.
#[tauri::command]
async fn add_custom_tag(
    state: tauri::State<'_, AppState>,
    name: String,
    threshold: f64,
) -> Result<i64, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("tag name is empty".to_string());
    }
    if !(0.01..=0.5).contains(&threshold) {
        return Err("threshold must be in [0.01, 0.5]".to_string());
    }
    log::info!("add_custom_tag name={} threshold={}", name, threshold);
    let guard = state.ai_engine.lock().await;
    let engine = guard
        .as_ref()
        .ok_or_else(|| "AI engine not ready".to_string())?;
    let vec = engine.embed_text(&name).map_err(|e| e.to_string())?;
    drop(guard);
    let id = state.db.add_custom_tag(&name, &vec, threshold)?;
    // Refresh the in-memory tag-vector cache only — no tagging tasks are
    // enqueued here; the user triggers the full pass via "AI Tagging".
    rebuild_tag_cache(state.db.clone(), state.ai_engine.clone(), state.tag_cache.clone()).await;
    Ok(id)
}

#[tauri::command]
async fn delete_custom_tag(
    state: tauri::State<'_, AppState>,
    id: i64,
) -> Result<(), String> {
    log::info!("delete_custom_tag id={}", id);
    // Deleting a tag definition must also remove that tag from every photo
    // (cards, tag search, counts) — not just the definition.
    let name = state.db.get_custom_tag_name(id)?;
    state.db.delete_custom_tag(id)?;
    if let Some(name) = name {
        log::info!("removing tag {:?} from all photos", name);
        state.db.remove_tag_everywhere(&name)?;
    }
    rebuild_tag_cache(state.db.clone(), state.ai_engine.clone(), state.tag_cache.clone()).await;
    Ok(())
}

/// Settings "clear tags" (confirm dialog on the frontend): full reset of
/// tag definitions AND all photo tag assignments.
#[tauri::command]
async fn clear_all_tags(state: tauri::State<'_, AppState>) -> Result<(), String> {
    log::info!("clear_all_tags");
    state.db.clear_all_tags()?;
    rebuild_tag_cache(state.db.clone(), state.ai_engine.clone(), state.tag_cache.clone()).await;
    Ok(())
}

#[tauri::command]
fn get_custom_tags(state: tauri::State<AppState>) -> Result<Vec<db::CustomTag>, String> {
    state.db.get_custom_tags()
}

/// Every tag name ever used (most-used first, no "unknown") — the picker
/// lists for the card edit dialog and the multi-select "add tag" panel.
#[tauri::command]
fn get_all_tags(state: tauri::State<AppState>) -> Result<Vec<String>, String> {
    state.db.get_all_tag_names()
}

/// Distinct lens names across the library (C-15 filter panel), sorted.
#[tauri::command]
fn get_lens_list(state: tauri::State<AppState>) -> Result<Vec<String>, String> {
    state.db.get_lens_list()
}

/// Multi-select "add tag" (C-13): append the given tags as MANUAL tags
/// (source=0) to every selected file. Existing tags are kept.
#[tauri::command]
fn add_tags_to_files(
    state: tauri::State<AppState>,
    file_ids: Vec<i64>,
    tags: Vec<String>,
) -> Result<usize, String> {
    if file_ids.is_empty() {
        return Err("no files selected".to_string());
    }
    let tags: Vec<String> = tags.into_iter().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect();
    if tags.is_empty() {
        return Err("no tags given".to_string());
    }
    log::info!("add_tags_to_files: {} files, tags {:?}", file_ids.len(), tags);
    state.db.add_manual_tags_batch(&file_ids, &tags)?;
    Ok(file_ids.len())
}

/// Multi-select "delete tags" (C-15.1): remove EVERY tag from the selected
/// files — text tags (manual + AI) AND color labels.
#[tauri::command]
fn clear_tags_from_files(
    state: tauri::State<AppState>,
    file_ids: Vec<i64>,
) -> Result<usize, String> {
    if file_ids.is_empty() {
        return Err("no files selected".to_string());
    }
    log::info!("clear_tags_from_files: {} files", file_ids.len());
    state.db.clear_all_tags_on_files(&file_ids)
}

/// Color labels (C-14): apply/remove ONE color to the selection (phone
/// gallery semantics — toggle). Colors are stored separately from text
/// tags. Returns whether all selected files carry the color afterwards.
#[tauri::command]
fn toggle_color_tag(
    state: tauri::State<AppState>,
    file_ids: Vec<i64>,
    color: String,
) -> Result<bool, String> {
    const COLORS: [&str; 7] = ["red", "orange", "yellow", "green", "blue", "purple", "reject"];
    if !COLORS.contains(&color.as_str()) {
        return Err(format!("invalid color: {color}"));
    }
    if file_ids.is_empty() {
        return Err("no files selected".to_string());
    }
    log::info!("toggle_color_tag: {} files, color={}", file_ids.len(), color);
    state.db.toggle_color_tag(&file_ids, &color)
}

/// Multi-select "Export" (C-19.10): copy the selected photos into a chosen
/// destination folder. Name collisions get a numeric suffix. Returns the
/// number of files copied.
#[tauri::command]
fn export_files(
    state: tauri::State<AppState>,
    file_ids: Vec<i64>,
    dest_dir: String,
) -> Result<usize, String> {
    if file_ids.is_empty() {
        return Err("no files selected".to_string());
    }
    let dest = std::path::PathBuf::from(&dest_dir);
    if !dest.is_dir() {
        return Err(format!("destination is not a folder: {dest_dir}"));
    }
    let mut copied = 0usize;
    for id in &file_ids {
        let rec = state
            .db
            .get_file_by_id(*id)?
            .ok_or_else(|| "file not found".to_string())?;
        let src = std::path::Path::new(&rec.path);
        if !src.is_file() {
            log::warn!("export: source missing: {}", rec.path);
            continue;
        }
        // Collision-safe destination name: name, name (1), name (2), ...
        let stem = std::path::Path::new(&rec.filename);
        let base = stem.file_stem().unwrap_or_default().to_string_lossy().to_string();
        let ext = stem.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
        let mut name = rec.filename.clone();
        let mut i = 1usize;
        let mut out = dest.join(&name);
        while out.exists() {
            name = format!("{base} ({i}){ext}");
            out = dest.join(&name);
            i += 1;
        }
        if let Err(e) = std::fs::copy(src, &out) {
            log::warn!("export: copy failed for {}: {e}", rec.path);
            continue;
        }
        copied += 1;
    }
    log::info!("export_files: copied {copied} files to {dest_dir}");
    Ok(copied)
}

/// Multi-select "Delete" (C-19.10, rejects page only): PERMANENTLY remove the
/// selected photos from disk AND the library. The frontend confirms first.
#[tauri::command]
fn delete_files(state: tauri::State<AppState>, file_ids: Vec<i64>) -> Result<usize, String> {
    if file_ids.is_empty() {
        return Err("no files selected".to_string());
    }
    let mut deleted = 0usize;
    for id in &file_ids {
        if let Ok(Some(rec)) = state.db.get_file_by_id(*id) {
            if let Err(e) = std::fs::remove_file(std::path::Path::new(&rec.path)) {
                if e.kind() != std::io::ErrorKind::NotFound {
                    log::warn!("delete: remove failed for {}: {e}", rec.path);
                }
            }
        }
        state.db.delete_file(*id)?;
        deleted += 1;
    }
    log::info!("delete_files: removed {deleted} files");
    Ok(deleted)
}

/// Manual "AI Tagging" (C-12): the ONLY way tagging starts. Enqueues a
/// full tag-list check (AITask::tag_all) for every file missing at least
/// one currently-defined custom tag — this covers newly added tags AND
/// files that were only indexed (or never indexed) since the last pass.
/// Returns how many files were queued.
#[tauri::command]
async fn run_ai_tagging(state: tauri::State<'_, AppState>) -> Result<usize, String> {
    if state.db.get_custom_tags()?.is_empty() {
        log::info!("run_ai_tagging: no tags defined — nothing to do");
        return Err("no tags defined".to_string());
    }
    let db = state.db.clone();
    let tx = state.ai_queue.clone();
    let epoch = state.ai_control.epoch();
    tauri::async_runtime::spawn_blocking(move || -> Result<usize, String> {
        let mut enqueued = 0usize;
        let mut offset = 0i64;
        loop {
            let batch = db.get_files_missing_any_tag(5000, offset)?;
            if batch.is_empty() {
                break;
            }
            for (fid, path) in &batch {
                let mut task = ai::queue::AITask::tag_all(*fid, path.clone(), epoch);
                // The channel (capacity 1000) can fill up while the consumer
                // waits for the engine — retry until space frees, so one
                // click never silently drops files from the pass.
                loop {
                    match tx.try_send(task) {
                        Ok(()) => break,
                        Err(tokio::sync::mpsc::error::TrySendError::Full(t)) => {
                            task = t; // got the task back — sleep and retry
                            std::thread::sleep(std::time::Duration::from_millis(100));
                        }
                        Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                            log::warn!("run_ai_tagging: AI queue closed — aborting enqueue");
                            return Ok(enqueued);
                        }
                    }
                }
                enqueued += 1;
            }
            offset += batch.len() as i64;
        }
        log::info!("AI tagging: enqueued {enqueued} files");
        Ok(enqueued)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Set the OS desktop wallpaper to the given image (right-click menu, C-15.2).
/// Windows: SystemParametersInfoW via raw FFI (no extra crates); macOS:
/// osascript/System Events (may prompt for accessibility permission);
/// Linux: gsettings (GNOME).
#[tauri::command]
fn set_wallpaper(path: String) -> Result<(), String> {
    log::info!("set_wallpaper {}", path);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::ffi::OsStrExt;
        let wide: Vec<u16> = std::ffi::OsStr::new(&path)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        const SPI_SETDESKWALLPAPER: u32 = 0x0014;
        const SPIF_UPDATEINIFILE: u32 = 0x01;
        const SPIF_SENDCHANGE: u32 = 0x02;
        unsafe extern "system" {
            fn SystemParametersInfoW(
                uiAction: u32,
                uiParam: u32,
                pvParam: *mut std::ffi::c_void,
                fWinIni: u32,
            ) -> i32;
        }
        let ok = unsafe {
            SystemParametersInfoW(
                SPI_SETDESKWALLPAPER,
                0,
                wide.as_ptr() as *mut std::ffi::c_void,
                SPIF_UPDATEINIFILE | SPIF_SENDCHANGE,
            )
        };
        if ok == 0 {
            return Err("SystemParametersInfoW failed (invalid image?)".to_string());
        }
    }
    #[cfg(target_os = "macos")]
    {
        let escaped = path.replace('\\', "\\\\").replace('"', "\\\"");
        let out = std::process::Command::new("osascript")
            .args([
                "-e",
                &format!(
                    "tell application \"System Events\" to set picture of every desktop to \"{escaped}\""
                ),
            ])
            .output()
            .map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(format!(
                "osascript failed: {}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let uri = format!("file://{}", path);
        let out = std::process::Command::new("gsettings")
            .args(["set", "org.gnome.desktop.background", "picture-uri", &uri])
            .output()
            .map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(format!(
                "gsettings failed: {}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }
    }
    Ok(())
}

/// Self-update detection (C-18): hash the running exe, fetch
/// tiol.netlify.app/version.json, compare. Never errors — see update.rs.
#[tauri::command]
async fn check_update() -> update::UpdateInfo {
    update::check_update().await
}

#[tauri::command]
fn get_ai_status(state: tauri::State<AppState>) -> Result<String, String> {
    let guard = state.ai_status.lock().unwrap();
    Ok(match guard.as_ref() {
        Some(ModelStatus::Locked(b)) => format!("locked:{b}"),
        Some(ModelStatus::Degraded(r)) => format!("degraded: {r}"),
        None => "unknown".to_string(),
    })
}

/// Switch the inference backend: "auto" (detect), "gpu", "cpu", "coreml"
/// (Apple CoreML; falls back to CPU elsewhere). Reloads the engine.
#[tauri::command]
fn set_ai_provider(state: tauri::State<AppState>, provider: String) -> Result<(), String> {
    if !matches!(provider.as_str(), "auto" | "gpu" | "cpu" | "coreml") {
        return Err("invalid provider (auto|gpu|cpu|coreml)".to_string());
    }
    log::info!("set_ai_provider {}", provider);
    state.db.set_setting("ai_provider", &provider)?;
    spawn_engine_load(
        state.model_dir.clone(),
        state.ai_engine.clone(),
        state.ai_status.clone(),
        state.db.clone(),
    );
    Ok(())
}

#[tauri::command]
async fn get_thumbnail(
    state: tauri::State<'_, AppState>,
    path: String,
) -> Result<String, String> {
    let app_dir = state.app_dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let start = std::time::Instant::now();
        let p = std::path::Path::new(&path);
        let meta = match std::fs::metadata(p) {
            Ok(m) => m,
            Err(e) => {
                log::warn!("thumb metadata failed for {}: {}", path, e);
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
            if let Err(e) = utils::generate_thumbnail(p, &thumb_path) {
                log::warn!("thumb gen failed for {}: {}", path, e);
                return Ok(String::new());
            }
            let elapsed = start.elapsed().as_millis();
            if elapsed > 10000 {
                log::warn!("thumb gen slow: {}ms for {}", elapsed, path);
            }
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

/// One-time migration of app data from the old identifier.
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
        // Thumbnails can be mid-write (frontend requests / prewarm workers):
        // retry with backoff before giving up — transient file locks on
        // Windows otherwise fail the whole clear.
        let mut last_err: Option<std::io::Error> = None;
        for attempt in 0..5u32 {
            if !cache.exists() {
                return Ok::<(), String>(());
            }
            match std::fs::remove_dir_all(&cache) {
                Ok(()) => return Ok::<(), String>(()),
                Err(e) => {
                    last_err = Some(e);
                    std::thread::sleep(std::time::Duration::from_millis(250 * (attempt + 1) as u64));
                }
            }
        }
        Err(last_err
            .map(|e| e.to_string())
            .unwrap_or_else(|| "cache clear failed".to_string()))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Frontend instrumentation: JS errors / rejected promises / thumbnail
/// failures reported from the webview. Always written to the log buffer so
/// the debug-mode panel (and stderr) can show what actually broke in the UI.
#[tauri::command]
fn report_js_event(kind: String, message: String) -> Result<(), String> {
    log::info!("[webview:{kind}] {message}");
    Ok(())
}

/// HH:MM:SS (UTC) for the log format.
fn now_hms() -> String {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{:02}:{:02}:{:02}", (d / 3600) % 24, (d / 60) % 60, d % 60)
}

/// Debug mode: persist the toggle and raise/lower the global log level so the
/// in-app log panel (get_logs) captures info+ lines only while enabled.
#[tauri::command]
fn set_debug_mode(state: tauri::State<AppState>, enabled: bool) -> Result<(), String> {
    state
        .db
        .set_setting("debug", if enabled { "1" } else { "0" })?;
    log::set_max_level(if enabled {
        log::LevelFilter::Info
    } else {
        log::LevelFilter::Error
    });
    log::info!("debug mode {}", if enabled { "on" } else { "off" });
    Ok(())
}

/// Recent log lines for the debug panel (newest first).
#[tauri::command]
fn get_logs(limit: Option<usize>) -> Vec<String> {
    logbuf::snapshot(limit.unwrap_or(200).min(1000))
}

/// Pin the ONNX Runtime dynamic library BEFORE any ort call: ort load-dynamic
/// resolves ORT_DYLIB_PATH -> exe-dir/<default name> -> bare name. The bare
/// fallback on Windows can pick up the INBOX `C:\Windows\system32\
/// onnxruntime.dll` (Win11 24H2 ships one) — a minimal build whose CPU EP
/// lacks ConvInteger kernels, producing confusing "Could not find an
/// implementation for ConvInteger" errors (C-11.2). Pinning the vendored
/// library next to the executable avoids that entirely.
fn pin_ort_dylib() {
    #[cfg(target_os = "windows")]
    const LIB_NAME: &str = "onnxruntime.dll";
    #[cfg(target_os = "macos")]
    const LIB_NAME: &str = "libonnxruntime.dylib";
    #[cfg(all(unix, not(target_os = "macos")))]
    const LIB_NAME: &str = "libonnxruntime.so";

    if std::env::var_os("ORT_DYLIB_PATH").is_some() {
        return; // explicit override (tests / manual setups)
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            // Candidate locations: next to the executable, and (macOS .app
            // convention) Contents/Frameworks.
            #[allow(unused_mut)] // macOS-only push
            let mut candidates = vec![dir.join(LIB_NAME)];
            #[cfg(target_os = "macos")]
            candidates.push(dir.join("../Frameworks").join(LIB_NAME));
            for candidate in candidates {
                if candidate.exists() {
                    std::env::set_var("ORT_DYLIB_PATH", &candidate);
                    log::info!("pinned ONNX Runtime: {}", candidate.display());
                    return;
                }
            }
            log::warn!(
                "{} not found next to the executable (or Contents/Frameworks on macOS) — \
                 ort will use its default search; on Windows this may load the system inbox \
                 copy. See BUILD.md §2.",
                LIB_NAME
            );
        }
    }
}

fn main() {
    pin_ort_dylib();
    logbuf::init();
    env_logger::Builder::new()
        .filter_level(log::LevelFilter::Info)
        .format(|buf, record| {
            let line = format!(
                "[{} {}] {}: {}",
                now_hms(),
                record.level(),
                record.target(),
                record.args()
            );
            logbuf::push(line.clone());
            writeln!(buf, "{line}")
        })
        .init();
    tauri::Builder::default()
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            let app_dir = app
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("."));
            migrate_old_data(&app_dir);
            std::fs::create_dir_all(&app_dir).ok();
            std::fs::create_dir_all(utils::cache_dir(&app_dir)).ok();
            let db_path = app_dir.join("db.sqlite");
            log::info!("db path {:?}", db_path);
            let db = Arc::new(Db::new(&db_path).expect("failed to init db"));
            // Restore persisted debug-mode log level.
            if db.get_setting("debug").ok().flatten().as_deref() == Some("1") {
                log::set_max_level(log::LevelFilter::Info);
            }
            // Embedding-pipeline version gate (C-09): the pooler_output fix
            // changed the embedding space — old embeddings are incompatible,
            // so wipe + re-index once.
            const EMBED_VERSION: &str = "pooler-v1";
            if db.get_setting("embed_version").ok().flatten().as_deref() != Some(EMBED_VERSION) {
                log::info!("embedding pipeline changed ({EMBED_VERSION}) — re-indexing all photos");
                db.reindex_embeddings()?;
                db.set_setting("embed_version", EMBED_VERSION)?;
            }
            // Self-heal: photos carrying BOTH a real tag and the "unknown"
            // sentinel (older bug) lose the sentinel.
            if let Ok(n) = db.cleanup_stray_unknown() {
                if n > 0 {
                    log::info!("cleaned {n} stray 'unknown' tags");
                }
            }
            // Reject-metrics rule-version gate (C-19.5): when the over/under
            // exposure or eyes thresholds change, cached metrics become
            // invalid — reset them once so the next pass recomputes.
            const EXPOSURE_RULE_VERSION: &str = "over=0.20,under=0.20/160,eyes=0.11";
            if db.get_setting("exposure_rule_version").ok().flatten().as_deref()
                != Some(EXPOSURE_RULE_VERSION)
            {
                log::info!("reject rules changed — resetting cached metrics");
                db.reset_reject_metrics()?;
                db.set_setting("exposure_rule_version", EXPOSURE_RULE_VERSION)?;
            }

            // ---- AI: model lock + download (ADD.md §4); engine loads after ----
            let model_dir = app
                .path()
                .app_cache_dir()
                .unwrap_or_else(|_| app_dir.clone())
                .join("models");
            log::info!("model dir: {:?}", model_dir);

            // ---- AI: queue + consumer (ADD.md §5) ----
            let (ai_tx, ai_rx) = tokio::sync::mpsc::channel::<ai::queue::AITask>(ai::queue::QUEUE_CAPACITY);
            let ai_engine: Arc<AsyncMutex<Option<AIEngine>>> = Arc::new(AsyncMutex::new(None));
            let ai_status: Arc<std::sync::Mutex<Option<ModelStatus>>> = Arc::new(std::sync::Mutex::new(None));
            let ai_control = Arc::new(ai::queue::AIControl::new());
            // User-defined tag vectors (text embeddings), shared with the
            // queue for zero-shot tag matching (MIGRATE1.md V3.0).
            let tag_cache: Arc<std::sync::RwLock<Vec<ai::engine::TagVec>>> =
                Arc::new(std::sync::RwLock::new(Vec::new()));
            // Set once the first cache build finished — the consumer waits
            // for it so no file is processed against an empty cache (C-11.1).
            let cache_ready: Arc<std::sync::atomic::AtomicBool> =
                Arc::new(std::sync::atomic::AtomicBool::new(false));
            let consumer_db = db.clone();
            let consumer_engine = ai_engine.clone();
            let consumer_tags = tag_cache.clone();
            let consumer_ready = cache_ready.clone();
            let consumer_control = ai_control.clone();
            let consumer_app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                ai::queue::run_consumer(
                    ai_rx,
                    consumer_db,
                    consumer_engine,
                    consumer_tags,
                    consumer_ready,
                    consumer_control,
                    Some(consumer_app),
                )
                .await;
            });

            // ---- AI: model lock + download (ADD.md §4); engine loads AFTER
            // models are verified (a first-time user has nothing downloaded:
            // loading before the download finishes would fail permanently —
            // C-11.3). When models already exist, verification is a quick
            // hash check, so this adds ~1-2s before the engine is ready.
            {
                let app_handle = app.handle().clone();
                let status_holder = ai_status.clone();
                let model_dir_task = model_dir.clone();
                let engine_holder = ai_engine.clone();
                let status_db = db.clone();
                tauri::async_runtime::spawn(async move {
                    let status = ai::downloader::init_models_async(model_dir_task.clone(), app_handle.clone()).await;
                    {
                        let mut s = status_holder.lock().unwrap();
                        *s = Some(status.clone());
                    }
                    ai::downloader::emit_status(&app_handle, &status);
                    if matches!(status, ModelStatus::Locked(_)) {
                        log::info!("models verified — loading engine");
                        spawn_engine_load(model_dir_task, engine_holder, status_holder, status_db);
                    }
                });
            }
            // Engine watchdog: if the engine still hasn't loaded, retry every
            // 30s for up to 10 minutes. Transient startup failures (CoreML
            // first compile, dylib timing, model verify races) self-heal
            // instead of leaving the whole queue blocked forever — the queue
            // waits for the engine, so "embedding never started" on a fresh
            // macOS install usually means the engine load failed (C-11.7).
            {
                let wh = ai_engine.clone();
                let ws = ai_status.clone();
                let wm = model_dir.clone();
                let wdb = db.clone();
                tauri::async_runtime::spawn(async move {
                    for _ in 0..20 {
                        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                        if wh.lock().await.is_some() {
                            return; // engine is up
                        }
                        log::warn!("engine not loaded yet — retrying engine load");
                        spawn_engine_load(wm.clone(), wh.clone(), ws.clone(), wdb.clone());
                    }
                    log::error!("engine watchdog gave up after 10 minutes — AI features offline (see settings model status / debug log)");
                });
            }

            // ---- window (hw decode + error page) ----
            let hw_args = match db.get_setting("hw_decode").ok().flatten().as_deref() {
                Some("1") => "--enable-gpu --ignore-gpu-blocklist --enable-accelerated-video-decode --enable-zero-copy",
                Some("0") => "--disable-gpu",
                _ => "",
            };
            log::info!("hw_decode browser args: {:?}", hw_args);
            if let Some(window_cfg) = app.config().app.windows.iter().find(|w| !w.create) {
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
                        // Accept the app pages AND the legacy/asset scheme
                        // ("tauri://localhost" appeared on macOS first-load,
                        // C-11.10) — only foreign URLs redirect to the error
                        // page.
                        let is_app_page = url.ends_with("index.html")
                            || url.ends_with("error.html")
                            || url.ends_with('/')
                            || url.starts_with("tauri://localhost")
                            || url.contains("tauri.localhost");
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

            // ---- state ----
            let state = AppState {
                db: db.clone(),
                ai: ai::MockSkill::new(),
                app_dir: app_dir.clone(),
                model_dir: model_dir.clone(),
                ai_queue: ai_tx,
                ai_control: ai_control.clone(),
                ai_engine: ai_engine.clone(),
                tag_cache: tag_cache.clone(),
                ai_status: ai_status.clone(),
                watcher: Arc::new(std::sync::Mutex::new(None)),
                reject_analysis_running: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            };
            // Engine loads AFTER models are verified (see the downloader
            // task above) — no eager load here, or a first-time user whose
            // models are still downloading would fail permanently (C-11.3).
            // Build the user-tag vector cache once the engine is ready
            // (MIGRATE1.md §2.2: tag vectors are cached in memory).
            {
                let cdb = db.clone();
                let cengine = ai_engine.clone();
                let ccache = tag_cache.clone();
                let cready = cache_ready.clone();
                tauri::async_runtime::spawn(async move {
                    for _ in 0..120 {
                        if cengine.lock().await.is_some() {
                            break;
                        }
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    }
                    rebuild_tag_cache(cdb, cengine, ccache).await;
                    // Release the consumer (even on an empty cache — the
                    // empty-tag-list case is handled inside process_one).
                    cready.store(true, std::sync::atomic::Ordering::SeqCst);
                });
            }

            // ---- startup scan (ADD.md §3.1) + watcher + prewarm + AI enqueue ----
            {
                let db_clone = db.clone();
                let handle = app.handle().clone();
                let prewarm_dir = app_dir.clone();
                let tx = state.ai_queue.clone();
                let db2 = db.clone();
                let epoch = state.ai_control.epoch();
                std::thread::spawn(move || {
                    let mut pending: Vec<i64> = Vec::new();
                    if let Ok(folders) = db_clone.get_folders() {
                        for f in folders {
                            if let Ok((_, _, mut p)) = scanner::scan_folder(&db_clone, f.id, &f.path) {
                                pending.append(&mut p);
                            } else {
                                log::error!("startup scan {} failed", f.path);
                            }
                        }
                    }
                    let _ = handle.emit("scan-complete", Vec::<(i64, usize, usize)>::new());
                    for fid in pending {
                        if let Ok(Some(rec)) = db2.get_file_by_id(fid) {
                            let _ = tx.try_send(ai::queue::AITask::new(fid, rec.path, epoch));
                        }
                    }
                    // Enqueue everything still pending AI processing (covers
                    // migration + unchanged files from earlier sessions).
                    if let Ok(files) = db2.get_pending_ai_files(5000) {
                        log::info!("enqueueing {} files for AI processing", files.len());
                        for (fid, path) in files {
                            let _ = tx.try_send(ai::queue::AITask::new(fid, path, epoch));
                        }
                    }
                    // NOTE: startup enqueues are INDEX tasks only (C-12) —
                    // file changes are embedded, never tagged. Tagging runs
                    // exclusively from the manual "AI Tagging" button
                    // (run_ai_tagging), which re-checks every file missing
                    // any current tag (new tags + new files included).
                    spawn_thumb_prewarm(prewarm_dir, db_clone);
                    spawn_exif_backfill(db2.clone());
                });
            }
            rebuild_watcher(&state, app.handle().clone());
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            add_folder,
            remove_folder,
            get_folders,
            get_photos,
            search_files,
            search_description,
            search,
            update_description,
            update_tags,
            get_file_tags,
            set_rating,
            set_rating_files,
            compute_reject_metrics,
            get_setting,
            set_setting,
            restart_app,
            report_renderer,
            reveal_in_folder,
            clear_cache,
            scan_folders,
            get_thumbnail,
            process_file,
            add_custom_tag,
            delete_custom_tag,
            get_custom_tags,
            get_all_tags,
            get_lens_list,
            add_tags_to_files,
            clear_tags_from_files,
            toggle_color_tag,
            export_files,
            delete_files,
            run_ai_tagging,
            clear_all_tags,
            get_ai_status,
            set_ai_provider,
            set_wallpaper,
            check_update,
            set_debug_mode,
            get_logs,
            report_js_event
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
