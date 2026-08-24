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
    Ok(files)
}
