use crate::db::Db;
use notify::{RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::Emitter;
use tokio::sync::mpsc;

pub struct FileWatcher {
    // Keep watchers alive
    _watchers: Vec<notify::RecommendedWatcher>,
    // Holds a channel sender so the debounce task's receiver never sees the
    // channel close (deliberately unread; the per-watcher callbacks also
    // hold senders, this one is extra insurance).
    #[allow(dead_code)]
    tx: mpsc::Sender<String>,
}

impl FileWatcher {
    pub fn start(
        db: Arc<Db>,
        folders: Vec<(i64, String)>,
        ai_queue: crate::ai::queue::AITaskSender,
        control: Arc<crate::ai::queue::AIControl>,
        app: tauri::AppHandle,
    ) -> Result<Self, String> {
        let (tx, mut rx) = mpsc::channel::<String>(32);
        let tx_clone = tx.clone();

        // Map path -> folder_id for debounce scan
        let path_to_id: Arc<Mutex<HashMap<String, i64>>> = Arc::new(Mutex::new(HashMap::new()));
        {
            let mut m = path_to_id.lock().unwrap();
            for (id, p) in &folders {
                m.insert(p.clone(), *id);
            }
        }

        // Spawn debounce task: 3s delay per LIMITS.md:113
        let db_clone = db.clone();
        let map_clone = path_to_id.clone();
        tauri::async_runtime::spawn(async move {
            let mut pending: HashMap<String, tauri::async_runtime::JoinHandle<()>> = HashMap::new();
            while let Some(changed_path) = rx.recv().await {
                // Find which folder this path belongs to (prefix match)
                let folder_opt = {
                    let m = map_clone.lock().unwrap();
                    // longest prefix wins
                    let mut best: Option<(String, i64)> = None;
                    for (fp, fid) in m.iter() {
                        if changed_path.starts_with(fp)
                            && best.as_ref().map_or(true, |(bp, _)| fp.len() > bp.len())
                        {
                            best = Some((fp.clone(), *fid));
                        }
                    }
                    best
                };
                if let Some((folder_path, folder_id)) = folder_opt {
                    // debounce: cancel previous handle for same folder
                    if let Some(h) = pending.remove(&folder_path) {
                        h.abort();
                    }
                    let db2 = db_clone.clone();
                    let fp = folder_path.clone();
                    let ai_tx = ai_queue.clone();
                    let control = control.clone();
                    let app = app.clone();
                    let handle = tauri::async_runtime::spawn(async move {
                        tokio::time::sleep(Duration::from_secs(3)).await;
                        log::info!("debounced scan for {}", fp);
                        match crate::scanner::scan_folder(&db2, folder_id, &fp) {
                            Ok((_, _, pending_ids)) => {
                                // Enqueue new/changed files for AI processing
                                // (embed + tag) — previously dropped here,
                                // so mid-session changes were never tagged
                                // until the next startup (C-10.4).
                                let epoch = control.epoch();
                                let mut queued = 0usize;
                                for fid in pending_ids {
                                    if let Ok(Some(rec)) = db2.get_file_by_id(fid) {
                                        if ai_tx
                                            .try_send(crate::ai::queue::AITask::new(
                                                fid, rec.path, epoch,
                                            ))
                                            .is_ok()
                                        {
                                            queued += 1;
                                        }
                                    }
                                }
                                if queued > 0 {
                                    log::info!("watcher: enqueued {queued} changed files for AI");
                                }
                                // EXIF for the new/changed files, off the scan
                                // path (C-19.8).
                                crate::spawn_exif_backfill(db2.clone());
                                // Let the frontend refresh lists/thumbnails.
                                let _ = app.emit("scan-complete", Vec::<(i64, usize, usize)>::new());
                            }
                            Err(e) => log::error!("scan error {}: {}", fp, e),
                        }
                    });
                    pending.insert(folder_path, handle);
                }
            }
        });

        let mut watchers = Vec::new();
        for (_, folder_path) in folders {
            let tx_inner = tx_clone.clone();
            let mut watcher = notify::recommended_watcher(move |res: Result<notify::Event, _>| {
                if let Ok(event) = res {
                    for p in event.paths {
                        let s = p.to_string_lossy().replace('\\', "/");
                        let _ = tx_inner.try_send(s);
                    }
                }
            })
            .map_err(|e| e.to_string())?;
            watcher
                .watch(Path::new(&folder_path), RecursiveMode::Recursive)
                .map_err(|e| e.to_string())?;
            watchers.push(watcher);
        }

        Ok(Self {
            _watchers: watchers,
            tx,
        })
    }
}
