//! AI task queue (ADD.md §5, MIGRATE1.md V3.0 / C-09, C-12): tokio mpsc
//! (capacity 1000), single consumer. Tasks come in two kinds (C-12):
//! - `Index`  (AITask::new): SigLIP embedding only — enqueued for new/changed
//!   files (startup scan, watcher, add/scan folders). Never touches tags.
//! - `TagAll` (AITask::tag_all): embedding (once) + match against ALL cached
//!   user-defined tag vectors -> file_tags (or "unknown") -> ai_processed=3.
//!   Enqueued only by the manual "AI Tagging" button (run_ai_tagging) for
//!   files missing at least one current tag.
//! Retries with 1s/5s/10s backoff, 100ms idle sleep. Holds tasks while the
//! engine is not ready.
//!
//! Folder invalidation: every task carries the queue `epoch` it was enqueued
//! with. When a folder is removed the epoch is bumped (AIControl::invalidate),
//! so stale tasks are skipped as soon as the consumer reaches them instead of
//! blocking newer work (the consumer never processes an old-epoch task).
//!
//! Resource control: only the shared SigLIP engine runs (on-demand); no
//! separate tagger model exists anymore. When no user tags are defined a
//! TagAll pass logs once and only embeds (no "unknown" spam on an empty list).

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

/// What a task should do with the file (C-12): file changes only index
/// (embed); the manual "AI Tagging" button runs a full tag match.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TaskKind {
    /// Embed only — new/changed files (startup scan, watcher, add/scan).
    /// Never writes tags or the "unknown" sentinel.
    Index,
    /// Embed (if missing) + match against ALL current tags — the manual
    /// "AI Tagging" button. Writes hits or "unknown", then ai_processed=3.
    TagAll,
}

pub struct AITask {
    pub file_id: i64,
    pub path: String, // display path
    epoch: u64,
    pub kind: TaskKind,
    /// TagAll only: the tag NAMES to match (None = all current tags).
    /// Set when the user checks specific tags on the Tags page (C-19.15).
    pub tag_names: Option<Vec<String>>,
}

impl AITask {
    /// Index-only task (new/changed files): embed, never tag.
    pub fn new(file_id: i64, path: String, epoch: u64) -> Self {
        Self { file_id, path, epoch, kind: TaskKind::Index, tag_names: None }
    }
    /// Full tag-list check against every photo missing any current tag
    /// (the "AI Tagging" button). `tag_names` limits the pass to those tags.
    pub fn tag_all(file_id: i64, path: String, epoch: u64, tag_names: Option<Vec<String>>) -> Self {
        Self { file_id, path, epoch, kind: TaskKind::TagAll, tag_names }
    }
}

#[derive(Clone, Serialize)]
pub struct AiProgress {
    pub done: u64,
    pub remaining: i64,
    /// True when the CURRENT work involves tag matching (TagAll tasks from
    /// the "AI Tagging" button) — false while only embedding/indexing runs
    /// (badge shows "Indexing" instead of "Tagging", C-11.4 / C-12).
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
    // Kind of the most recently processed task — drives the badge label
    // ("Tagging" for TagAll passes, "Indexing" for plain file indexing).
    // Assigned from the first task before any read.
    let mut last_tagging;
    // Log "nothing to tag" once per TagAll pass when no user tags are defined.
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
        last_tagging = task.kind == TaskKind::TagAll;
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
                use tauri::Emitter;
                let _ = app.emit(
                    "ai-queue-status",
                    AiProgress { done: processed, remaining, tagging: last_tagging },
                );
                last_progress = std::time::Instant::now();
            }
        }
    }
}

/// One task. Two kinds (C-12):
/// - Index: SigLIP embedding only — never touches tags (file changes are
///   indexed, NOT tagged; tagging is manual via the "AI Tagging" button).
/// - TagAll: embedding (skipped when stored) + match against ALL cached user
///   tags; a photo matching nothing gets an "unknown" tag (source=1) so a
///   later pass knows it was already checked.
async fn process_one(
    engine: &AIEngine,
    tag_cache: &Arc<std::sync::RwLock<Vec<TagVec>>>,
    db: &Db,
    task: &AITask,
    empty_warned: &mut bool,
) -> Result<(), String> {
    let path = std::path::Path::new(&task.path);

    // ---- Index: embed only (new/changed files). Existing tags and the
    // "unknown" sentinel are left exactly as they are.
    if task.kind == TaskKind::Index {
        if !db.has_embedding(task.file_id).unwrap_or(false) {
            match engine.embed_image(path) {
                Ok(vec) => {
                    db.update_embedding(task.file_id, &vec)?;
                }
                Err(e) => {
                    log::debug!("embed_image failed for {}: {}", task.path, e);
                    db.set_ai_processed(task.file_id, 2)?;
                    return Ok(());
                }
            }
        }
        db.set_ai_processed(task.file_id, 3)?;
        return Ok(());
    }

    // ---- TagAll: full tag-list check (manual "AI Tagging" button).
    // 1. SigLIP image embedding — skipped when the file already has one
    // (files re-enqueued for tagging only).
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
    // -> embed only (files stay untagged; a later pass after tags are added
    // re-enqueues them — never spam "unknown" on an empty list).
    let tagvecs: Vec<TagVec> = match tag_cache.read() {
        Ok(g) => g.clone(),
        Err(_) => Vec::new(),
    };
    // C-19.15: a checked subset of tags limits the pass (None = all).
    let tagvecs: Vec<TagVec> = match &task.tag_names {
        Some(names) => tagvecs
            .into_iter()
            .filter(|tv| names.iter().any(|n| n == &tv.name))
            .collect(),
        None => tagvecs,
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
        // All tags were deleted after this pass was enqueued — mark done
        // without any tag check (and without "unknown" spam).
        log::info!("AI tagging: no tags defined — nothing to match");
    }

    // 3. Done.
    db.set_ai_processed(task.file_id, 3)?;
    Ok(())
}
