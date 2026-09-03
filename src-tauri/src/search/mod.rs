//! Search module (ADD.md §6): dual path — tag search and semantic search.

use crate::ai::engine::AIEngine;
use crate::db::{Db, FileRecord};
use crate::error::{AppError, Result};

/// Tag search: exact (NOCASE) -> contains -> empty. DB handles the tiering.
pub fn tag_search(db: &Db, query: &str) -> Result<Vec<FileRecord>> {
    db.tag_search(query).map_err(AppError::Db)
}

/// Semantic search (SigLIP): text embedding -> cosine similarity against all
/// image embeddings -> top 500. text-encoder failure is a hard error (no
/// fallback, ADD.md §6.2). Each result carries its similarity score in
/// `FileRecord::score` (None elsewhere) — debug-mode confidence badges.
pub fn semantic_search(db: &Db, engine: &AIEngine, query: &str) -> Result<Vec<FileRecord>> {
    let text_vec = engine
        .embed_text(query)
        .map_err(|e| AppError::Search(format!("text embedding failed: {e}")))?;

    let embeddings = db.get_embeddings().map_err(AppError::Db)?;
    let mut scored: Vec<(i64, f32)> = Vec::with_capacity(embeddings.len());
    for (id, vec) in &embeddings {
        if vec.len() != text_vec.len() {
            continue; // dimensionality mismatch — skip
        }
        let mut dot = 0f32;
        for (a, b) in vec.iter().zip(text_vec.iter()) {
            dot += a * b;
        }
        scored.push((*id, dot));
    }
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(500);
    let ids: Vec<i64> = scored.iter().map(|(id, _)| *id).collect();
    let mut files = db.get_files_by_ids(&ids).map_err(AppError::Db)?;
    // get_files_by_ids preserves the requested order — zip scores back on.
    for (file, (_, score)) in files.iter_mut().zip(scored.iter()) {
        file.score = Some(*score);
    }
    // C-19.23: attach same-stem RAW twins (any folder). RAW twins are never
    // embedded, so they can't be semantic hits on their own — but the user
    // expects BOTH files of a RAW+JPEG pair to show up in search results;
    // the frontend "hide duplicate RAW" toggle filters them back out. The
    // twin carries its JPEG's score so ordering/UI stay coherent.
    if !files.is_empty() {
        let twins = db.raw_twins_of(&files).map_err(AppError::Db)?;
        if !twins.is_empty() {
            let mut score_by_stem: std::collections::HashMap<String, f32> =
                std::collections::HashMap::new();
            for f in &files {
                if let Some(stem) = std::path::Path::new(&f.path)
                    .file_stem()
                    .and_then(|s| s.to_str())
                {
                    score_by_stem
                        .entry(stem.to_lowercase())
                        .or_insert(f.score.unwrap_or(0.0));
                }
            }
            let mut twins = twins;
            for t in twins.iter_mut() {
                if let Some(stem) = std::path::Path::new(&t.path)
                    .file_stem()
                    .and_then(|s| s.to_str())
                {
                    if let Some(s) = score_by_stem.get(&stem.to_lowercase()) {
                        t.score = Some(*s);
                    }
                }
            }
            files.extend(twins);
        }
    }
    Ok(files)
}
