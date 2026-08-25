//! AI task queue (ADD.md §5, MIGRATE1.md V3.0 / C-09): tokio mpsc
//! (capacity 1000), single consumer. Each task: SigLIP embedding (once) +
//! match against user-defined tag vectors -> file_tags (or "unknown") ->
//! ai_processed=3. Retries with 1s/5s/10s backoff, 100ms idle sleep.
//! Holds tasks while the engine is not ready.
//!
//! Folder invalidation: every task carries the queue `epoch` it was enqueued
//! with. When a folder is removed the epoch is bumped (AIControl::invalidate),
//! so stale tasks are skipped as soon as the consumer reaches them instead of
//! blocking newer work (the consumer never processes an old-epoch task).
//!
//! Resource control: only the shared SigLIP engine runs (on-demand); no
//! separate tagger model exists anymore. When no user tags are defined the
//! matching step is skipped entirely (no "unknown" spam on an empty list).

use crate::ai::engine::{cosine, AIEngine, TagVec};
use crate::db::Db;
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex};

pub const QUEUE_CAPACITY: usize = 1000;

/// Shared control knob for the queue. `epoch` counts folder invalidations;
/// tasks enqueued before an invalidation carry a lower epoch and are skipped.
#[derive(Default)]
pub struct AIControl {
    epoch: AtomicU64,
}

impl AIControl {
    pub fn new() -> Self {
        Self::default()
    }
    /// Current epoch for newly enqueued tasks.
    pub fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::SeqCst)
    }
    /// Bump the epoch: every task enqueued so far becomes stale.
    pub fn invalidate(&self) {
        self.epoch.fetch_add(1, Ordering::SeqCst);
    }
}

pub struct AITask {
    pub file_id: i64,
    pub path: String, // display path
    epoch: u64,
    /// When Some, ONLY this tag is checked against the photo (new-tag flow:
    /// existing tags are never re-evaluated). None = full tag-list check
    /// (new/changed files).
    pub tag: Option<TagVec>,
}

impl AITask {
    /// Full tag-list check (new/changed files).
    pub fn new(file_id: i64, path: String, epoch: u64) -> Self {
        Self { file_id, path, epoch, tag: None }
    }
    /// Check only the given tag (added later — multi-label without
    /// re-evaluating existing tags).
    pub fn with_tag(file_id: i64, path: String, epoch: u64, tag: TagVec) -> Self {
        Self { file_id, path, epoch, tag: Some(tag) }
    }
}

#[derive(Clone, Serialize)]
pub struct AiProgress {
    pub done: u64,
    pub remaining: i64,
    /// True when the current work involves tag matching (user tags exist) —
    /// false while only embedding/indexing runs (badge shows "Indexing"
    /// instead of "Tagging", C-11.4).
    pub tagging: bool,
}

pub type AITaskSender = mpsc::Sender<AITask>;

#[allow(clippy::too_many_arguments)]
pub async fn run_consumer(
    mut rx: mpsc::Receiver<AITask>,
    db: Arc<Db>,
    engine: Arc<Mutex<Option<AIEngine>>>,
    tag_cache: Arc<std::sync::RwLock<Vec<TagVec>>>,
    cache_ready: Arc<std::sync::atomic::AtomicBool>,
    control: Arc<AIControl>,
    app: Option<tauri::AppHandle>,
) {
    let mut processed: u64 = 0;
    // Throttle for the floating progress badge: emit at most every 300ms,
    // but ALWAYS emit when the queue empties (badge hides).
    let mut last_progress = std::time::Instant::now() - Duration::from_secs(1);
    // Log "embedding only" once per run when no user tags are defined.
    let mut empty_warned = false;
    // Wait for the tag cache to be built ONCE before processing anything:
    // files processed in the gap between engine-ready and cache-ready would
    // be marked done with an empty cache — no tag check, no "unknown", and
    // never revisited (C-11.1 race fix).
    loop {
        if cache_ready.load(std::sync::atomic::Ordering::SeqCst) {
            break;
        }
        if rx.is_closed() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    while let Some(task) = rx.recv().await {
        // Folder invalidated since this task was enqueued? Skip it (and any
        // other stale tasks already in the channel) without touching the file.
        let current = control.epoch();
        if task.epoch < current {
            continue;
        }

        // Wait for the engine to become available (model download in progress).
        // The task is never dropped while models are still downloading.
        loop {
            if engine.lock().await.is_some() {
                break;
            }
            if rx.is_closed() {
                return;
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }

        let mut attempts = 0;
        let delays = [1u64, 5, 10];
        loop {
            let guard = engine.lock().await;
            let result = match guard.as_ref() {
                Some(eng) => process_one(eng, &tag_cache, &db, &task, &mut empty_warned).await,
                None => Err("engine not ready".to_string()),
            };
            drop(guard);
            match result {
                Ok(()) => break,
                Err(e) => {
                    attempts += 1;
                    if attempts > 3 {
                        log::error!("AI task failed permanently for file {} ({}): {}", task.file_id, task.path, e);
                        break;
                    }
                    let delay = delays[(attempts - 1).min(2)];
                    log::warn!("AI task failed (attempt {attempts}) for {}: {}; retrying in {delay}s", task.path, e);
                    tokio::time::sleep(Duration::from_secs(delay)).await;
                }
            }
        }
        // Idle throttle (ADD.md §5): low CPU when the queue is quiet.
        tokio::time::sleep(Duration::from_millis(100)).await;
        processed += 1;
        if processed % 20 == 0 {
            log::info!("AI queue: {processed} files processed");
        }
        // Progress event for the floating badge: remaining = tasks still in
        // the channel (new tasks pushed meanwhile bump it — the badge shows
        // the growing backlog live).
        if let Some(app) = &app {
            let remaining = rx.len() as i64;
            let emit = remaining == 0 || last_progress.elapsed() >= Duration::from_millis(300);
            if emit {
                let tagging = !tag_cache
                    .read()
                    .map(|g| g.is_empty())
                    .unwrap_or(true);
                use tauri::Emitter;
                let _ = app.emit(
                    "ai-queue-status",
                    AiProgress { done: processed, remaining, tagging },
                );
                last_progress = std::time::Instant::now();
            }
        }
    }
}

/// One task: SigLIP embedding (skipped when already stored) + user-tag
/// matching. Files are marked ai_processed=3 when done; a photo that matches
/// no defined tag gets an "unknown" tag (source=1) so it is never re-detected.
async fn process_one(
    engine: &AIEngine,
    tag_cache: &Arc<std::sync::RwLock<Vec<TagVec>>>,
    db: &Db,
    task: &AITask,
    empty_warned: &mut bool,
) -> Result<(), String> {
    let path = std::path::Path::new(&task.path);

    // Single-tag check (a tag added later): evaluate ONLY that tag against
    // the photo. Existing tags are never touched, no "unknown" is written
    // (the photo may already carry other tags).
    if let Some(tv) = &task.tag {
        let vec = match db.get_embedding(task.file_id)? {
            Some(v) => v,
            None => match engine.embed_image(path) {
                Ok(v) => {
                    db.update_embedding(task.file_id, &v)?;
                    v
                }
                Err(e) => {
                    log::debug!("embed_image failed for {}: {}", task.path, e);
                    db.set_ai_processed(task.file_id, 2)?;
                    return Ok(());
                }
            },
        };
        let sim = cosine(&vec, &tv.vec);
        if sim as f64 > tv.threshold {
            // A real match must displace the "unknown" sentinel (it was
            // written by an earlier full check) — otherwise the photo ends
            // up with BOTH the tag and "unknown" (C-10.5).
            db.clear_unknown_tag(task.file_id)?;
            db.set_file_tags(task.file_id, &[(tv.name.clone(), sim)], 1)?;
            log::info!(
                "tag {} {}: {}={:.3} (new tag)",
                task.file_id,
                task.path,
                tv.name,
                sim
            );
        } else {
            log::info!(
                "tag {} {}: {} no match ({:.3} < {})",
                task.file_id,
                task.path,
                tv.name,
                sim,
                tv.threshold
            );
        }
        db.set_ai_processed(task.file_id, 3)?;
        return Ok(());
    }

    // 1. SigLIP image embedding — skipped when the file already has one
    // (legacy files re-enqueued for tagging only).
    let mut img_vec: Option<Vec<f32>> = None;
    if !db.has_embedding(task.file_id).unwrap_or(false) {
        match engine.embed_image(path) {
            Ok(vec) => {
                db.update_embedding(task.file_id, &vec)?;
                img_vec = Some(vec);
            }
            Err(e) => {
                log::debug!("embed_image failed for {}: {}", task.path, e);
                db.set_ai_processed(task.file_id, 2)?;
                return Ok(());
            }
        }
    }

    // 2. Match against the cached user-defined tag vectors. No tags defined
    // -> skip (files stay untagged and are re-enqueued on later startups
    // until the user defines tags — never spam "unknown" on an empty list).
    let tagvecs: Vec<TagVec> = match tag_cache.read() {
        Ok(g) => g.clone(),
        Err(_) => Vec::new(),
    };
    if !tagvecs.is_empty() {
        // The "unknown" sentinel must never linger once re-tagging runs —
        // drop it first, then write fresh matches (or re-insert it).
        db.clear_unknown_tag(task.file_id)?;
        // Use the stored embedding when present — re-running the vision
        // encoder just to re-tag is wasted work.
        let vec = match img_vec {
            Some(v) => v,
            None => match db.get_embedding(task.file_id)? {
                Some(v) => v,
                None => engine.embed_image(path).map_err(|e| e.to_string())?,
            },
        };
        let mut hits: Vec<(String, f32)> = Vec::new();
        for tv in &tagvecs {
            let sim = cosine(&vec, &tv.vec);
            if sim as f64 > tv.threshold {
                hits.push((tv.name.clone(), sim));
            }
        }
        hits.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        if hits.is_empty() {
            db.set_file_tags(task.file_id, &[("unknown".to_string(), 1.0)], 1)?;
            // Debug aid: show WHY nothing matched (best score vs threshold).
            let best = tagvecs
                .iter()
                .map(|tv| cosine(&vec, &tv.vec))
                .fold(f32::NEG_INFINITY, f32::max);
            let thresh: Vec<String> = tagvecs
                .iter()
                .map(|tv| format!("{}:{}", tv.name, tv.threshold))
                .collect();
            log::info!(
                "tag {} {}: no match (best {best:.3}, tags [{}]) -> unknown",
                task.file_id,
                task.path,
                thresh.join(", ")
            );
        } else {
            let top: Vec<String> = hits
                .iter()
                .take(5)
                .map(|(n, s)| format!("{n}={s:.3}"))
                .collect();
            log::info!("tag {} {}: {}", task.file_id, task.path, top.join(", "));
            db.set_file_tags(task.file_id, &hits, 1)?;
        }
    } else if !*empty_warned {
        *empty_warned = true;
        // First launch / no tags yet: embedding only — the file is marked
        // done WITHOUT any tag check (and without "unknown" spam). Adding a
        // tag later re-checks it (single-tag flow).
        log::info!("no user tags defined — embedding only, tagging skipped");
    }

    // 3. Done.
    db.set_ai_processed(task.file_id, 3)?;
    Ok(())
}
