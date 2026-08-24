//! AI layer (ADD.md §4/§5): model lock + downloader + inference engine + queue.
pub mod downloader;
pub mod engine;
pub mod model_lock;
pub mod queue;

use crate::db::{Db, FileRecord};
use std::collections::HashMap;

/// Mock AI search per LIMITS.md:116-119
/// Hard-coded keyword mappings (Chinese + English)
pub struct MockSkill {
    map: HashMap<String, Vec<String>>,
}

impl MockSkill {
    pub fn new() -> Self {
        let mut map = HashMap::new();
        map.insert(
            "风景".to_string(),
            vec![
                "landscape".to_string(),
                "mountain".to_string(),
                "scenery".to_string(),
            ],
        );
        map.insert("猫".to_string(), vec!["cat".to_string()]);
        map.insert("狗".to_string(), vec!["dog".to_string()]);
        map.insert(
            "人像".to_string(),
            vec!["portrait".to_string(), "people".to_string()],
        );
        map.insert("夜景".to_string(), vec!["night".to_string()]);
        map.insert(
            "海".to_string(),
            vec!["sea".to_string(), "ocean".to_string(), "beach".to_string()],
        );
        // English pass-through
        map.insert("landscape".to_string(), vec!["landscape".to_string()]);
        map.insert("cat".to_string(), vec!["cat".to_string()]);
        Self { map }
    }

    /// Expand query into keywords, then search DB with LIKE fallback
    pub fn search(&self, db: &Db, query: &str) -> Result<Vec<FileRecord>, String> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return db.get_photos(None);
        }
        // Try mapping
        let mut keywords = Vec::new();
        if let Some(mapped) = self.map.get(&q) {
            keywords.extend(mapped.clone());
        }
        keywords.push(q.clone());

        // Collect union of LIKE results
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for kw in keywords {
            let res = db.search_files(&kw)?;
            for r in res {
                if seen.insert(r.path.clone()) {
                    out.push(r);
                }
            }
        }
        // If nothing found and query was mapped, fallback to original LIKE already done
        Ok(out)
    }
}

impl Default for MockSkill {
    fn default() -> Self {
        Self::new()
    }
}
