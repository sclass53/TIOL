use rusqlite::{params, Connection, OptionalExtension, Result as SqliteResult};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Folder {
    pub id: i64,
    pub path: String, // display path (real case) — what the UI shows
    pub created_at: i64,
    pub photo_count: i64,
    pub last_scan_time: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRecord {
    pub id: i64,
    pub folder_id: i64,
    pub path: String, // display path (real case) — used for IO/thumbnails
    pub filename: String,
    pub size: i64,
    pub mtime: i64,
    pub created_at: i64,
    pub description: String,
    pub ai_processed: i64,
    /// AI similarity score (semantic search only). None elsewhere — the
    /// frontend shows it as a confidence badge when debug mode is enabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f32>,
    /// Tag names for this file (manual + auto), in insertion order.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Color labels (C-14) — one of red/orange/yellow/green/blue/purple,
    /// stored separately from text tags; a file can carry several.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub colors: Vec<String>,
    /// EXIF lens model (C-15), e.g. "EF50mm f/1.8 STM". None = not in EXIF.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lens: Option<String>,
    /// EXIF focal length in mm (C-15). None = not in EXIF.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focal_length: Option<f64>,
}

/// Shared SELECT column list for `files` (no alias) — column 9 is the
/// comma-joined tag names (NULL when none), column 10 the comma-joined
/// color labels, columns 11/12 the EXIF lens + focal length (C-15).
const FILE_COLS: &str = "id, folder_id, COALESCE(display_path, path), filename, size, mtime, created_at, description, ai_processed, \
    (SELECT GROUP_CONCAT(t.name, ',') FROM file_tags ft JOIN tags t ON t.id = ft.tag_id WHERE ft.file_id = files.id), \
    (SELECT GROUP_CONCAT(color, ',') FROM color_tags ct WHERE ct.file_id = files.id), \
    lens, focal_length";
/// Same, for queries aliasing the table as `f`.
const FILE_COLS_F: &str = "f.id, f.folder_id, COALESCE(f.display_path, f.path), f.filename, f.size, f.mtime, f.created_at, f.description, f.ai_processed, \
    (SELECT GROUP_CONCAT(t.name, ',') FROM file_tags ft JOIN tags t ON t.id = ft.tag_id WHERE ft.file_id = f.id), \
    (SELECT GROUP_CONCAT(color, ',') FROM color_tags ct WHERE ct.file_id = f.id), \
    f.lens, f.focal_length";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomTag {
    pub id: i64,
    pub name: String,
    pub threshold: f64,
    pub ref_count: i64,
    pub enabled: i64,
    /// Photos currently carrying this tag (for the settings UI).
    #[serde(default)]
    pub photo_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileTag {
    pub name: String,
    pub confidence: f64,
    pub source: i64, // 0=manual, 1=AI (DeepDanbooru)
}

pub struct Db {
    conn: Mutex<Connection>,
}

impl Db {
    pub fn new<P: AsRef<Path>>(db_path: P) -> Result<Self, String> {
        if let Some(parent) = db_path.as_ref().parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let conn = Connection::open(db_path.as_ref()).map_err(|e| e.to_string())?;
        Self::init_schema(&conn).map_err(|e| e.to_string())?;
        Self::migrate(&conn).map_err(|e| e.to_string())?;
        let _ = conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;");
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn init_schema(conn: &Connection) -> SqliteResult<()> {
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS folders (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                path TEXT UNIQUE NOT NULL,
                display_path TEXT,
                created_at INTEGER NOT NULL,
                last_scan_time INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS files (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                folder_id INTEGER NOT NULL,
                path TEXT UNIQUE NOT NULL,
                display_path TEXT,
                filename TEXT NOT NULL,
                size INTEGER NOT NULL,
                mtime INTEGER NOT NULL,
                created_at INTEGER NOT NULL,
                description TEXT NOT NULL DEFAULT '',
                embedding BLOB,
                ai_processed INTEGER NOT NULL DEFAULT 0,
                FOREIGN KEY(folder_id) REFERENCES folders(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_files_folder ON files(folder_id);
            CREATE INDEX IF NOT EXISTS idx_files_path ON files(path);
            CREATE INDEX IF NOT EXISTS idx_files_mtime ON files(mtime DESC);
            CREATE TABLE IF NOT EXISTS tags (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT UNIQUE NOT NULL COLLATE NOCASE
            );
            CREATE INDEX IF NOT EXISTS idx_tag_name ON tags(name COLLATE NOCASE);
            CREATE TABLE IF NOT EXISTS file_tags (
                file_id INTEGER NOT NULL,
                tag_id INTEGER NOT NULL,
                confidence REAL DEFAULT 1.0,
                source INTEGER DEFAULT 0,
                PRIMARY KEY(file_id, tag_id),
                FOREIGN KEY(file_id) REFERENCES files(id) ON DELETE CASCADE,
                FOREIGN KEY(tag_id) REFERENCES tags(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_file_tags_tag ON file_tags(tag_id);
            CREATE INDEX IF NOT EXISTS idx_file_tags_file ON file_tags(file_id);
            CREATE TABLE IF NOT EXISTS custom_tags (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT UNIQUE NOT NULL,
                embedding BLOB NOT NULL,
                threshold REAL DEFAULT 0.25,
                ref_count INTEGER DEFAULT 0,
                enabled INTEGER DEFAULT 1
            );
            CREATE TABLE IF NOT EXISTS color_tags (
                file_id INTEGER NOT NULL,
                color TEXT NOT NULL,
                PRIMARY KEY(file_id, color),
                FOREIGN KEY(file_id) REFERENCES files(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_color_tags_color ON color_tags(color);
            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            "#,
        )
    }

    /// Migrate older demo DBs: add columns, backfill display_path, and
    /// lowercase the storage key (ADD.md §9.1).
    fn migrate(conn: &Connection) -> SqliteResult<()> {
        let cols = |table: &str| -> SqliteResult<Vec<String>> {
            let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(1))?;
            rows.collect()
        };
        let fcols = cols("folders")?;
        if !fcols.iter().any(|c| c == "last_scan_time") {
            conn.execute_batch(
                "ALTER TABLE folders ADD COLUMN last_scan_time INTEGER NOT NULL DEFAULT 0;",
            )?;
        }
        if !fcols.iter().any(|c| c == "display_path") {
            conn.execute_batch("ALTER TABLE folders ADD COLUMN display_path TEXT;")?;
        }
        let filecols = cols("files")?;
        if !filecols.iter().any(|c| c == "display_path") {
            conn.execute_batch("ALTER TABLE files ADD COLUMN display_path TEXT;")?;
        }
        if !filecols.iter().any(|c| c == "embedding") {
            conn.execute_batch("ALTER TABLE files ADD COLUMN embedding BLOB;")?;
        }
        if !filecols.iter().any(|c| c == "ai_processed") {
            conn.execute_batch("ALTER TABLE files ADD COLUMN ai_processed INTEGER NOT NULL DEFAULT 0;")?;
        }
        if !filecols.iter().any(|c| c == "lens") {
            conn.execute_batch("ALTER TABLE files ADD COLUMN lens TEXT;")?;
        }
        if !filecols.iter().any(|c| c == "focal_length") {
            conn.execute_batch("ALTER TABLE files ADD COLUMN focal_length REAL;")?;
        }
        if !filecols.iter().any(|c| c == "exif_checked") {
            conn.execute_batch("ALTER TABLE files ADD COLUMN exif_checked INTEGER NOT NULL DEFAULT 0;")?;
        }
        // idx_files_ai needs the column to exist, so it is created after migrate.
        conn.execute_batch("CREATE INDEX IF NOT EXISTS idx_files_ai ON files(ai_processed);")?;
        let ftcols = cols("file_tags")?;
        if !ftcols.iter().any(|c| c == "confidence") {
            conn.execute_batch("ALTER TABLE file_tags ADD COLUMN confidence REAL DEFAULT 1.0;")?;
        }
        if !ftcols.iter().any(|c| c == "source") {
            conn.execute_batch("ALTER TABLE file_tags ADD COLUMN source INTEGER DEFAULT 0;")?;
        }
        conn.execute_batch(
            "UPDATE folders SET display_path = path WHERE display_path IS NULL OR display_path = '';
             UPDATE files SET display_path = path WHERE display_path IS NULL OR display_path = '';
             UPDATE OR IGNORE folders SET path = lower(path) WHERE path != lower(path);
             UPDATE OR IGNORE files SET path = lower(path) WHERE path != lower(path);",
        )?;
        Ok(())
    }

    pub fn add_folder(&self, path: &str) -> Result<i64, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let key = crate::utils::normalize_storage_path(path);
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT OR IGNORE INTO folders(path, display_path, created_at, last_scan_time) VALUES(?1, ?2, ?3, 0)",
            params![key, path, now],
        )
        .map_err(|e| e.to_string())?;
        let id: i64 = conn
            .query_row("SELECT id FROM folders WHERE path=?1", params![key], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        Ok(id)
    }

    pub fn remove_folder(&self, id: i64) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM folders WHERE id=?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn get_folders(&self) -> Result<Vec<Folder>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                r#"SELECT f.id, COALESCE(f.display_path, f.path) AS path, f.created_at,
                          f.last_scan_time, COUNT(fi.id) as cnt
                   FROM folders f LEFT JOIN files fi ON fi.folder_id = f.id
                   GROUP BY f.id ORDER BY f.created_at DESC"#,
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok(Folder {
                    id: r.get(0)?,
                    path: r.get(1)?,
                    created_at: r.get(2)?,
                    last_scan_time: r.get(3)?,
                    photo_count: r.get(4)?,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| e.to_string())?);
        }
        Ok(out)
    }

    /// (id, storage key) pairs — used by the file watcher.
    pub fn get_folder_keys(&self) -> Result<Vec<(i64, String)>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT id, path FROM folders")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| e.to_string())?);
        }
        Ok(out)
    }

    pub fn update_last_scan_time(&self, folder_id: i64, ts: i64) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE folders SET last_scan_time=?1 WHERE id=?2",
            params![ts, folder_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn get_photos(&self, folder_id: Option<i64>) -> Result<Vec<FileRecord>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let (sql, param): (String, Option<i64>) = match folder_id {
            Some(fid) => (
                format!("SELECT {FILE_COLS} FROM files WHERE folder_id=?1 ORDER BY mtime DESC LIMIT 2000"),
                Some(fid),
            ),
            None => (
                format!("SELECT {FILE_COLS} FROM files ORDER BY mtime DESC LIMIT 2000"),
                None,
            ),
        };
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let rows: Vec<FileRecord> = if let Some(fid) = param {
            let mapped = stmt
                .query_map(params![fid], map_file)
                .map_err(|e| e.to_string())?;
            mapped
                .collect::<SqliteResult<Vec<_>>>()
                .map_err(|e| e.to_string())?
        } else {
            let mapped = stmt.query_map([], map_file).map_err(|e| e.to_string())?;
            mapped
                .collect::<SqliteResult<Vec<_>>>()
                .map_err(|e| e.to_string())?
        };
        Ok(rows)
    }

    pub fn get_file_by_id(&self, id: i64) -> Result<Option<FileRecord>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let rec = conn
            .query_row(
                &format!("SELECT {FILE_COLS} FROM files WHERE id=?1"),
                params![id],
                map_file,
            )
            .optional()
            .map_err(|e| e.to_string())?;
        Ok(rec)
    }

    pub fn search_files(&self, query: &str) -> Result<Vec<FileRecord>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let like = format!("%{}%", query);
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {FILE_COLS} FROM files WHERE filename LIKE ?1 OR COALESCE(display_path, path) LIKE ?1 ORDER BY mtime DESC LIMIT 500"
            ))
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![like], map_file)
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| e.to_string())?);
        }
        Ok(out)
    }

    /// Search files by description (LIKE, case-insensitive via SQLite default)
    pub fn search_description(&self, query: &str) -> Result<Vec<FileRecord>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let like = format!("%{}%", query);
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {FILE_COLS} FROM files WHERE description LIKE ?1 ORDER BY mtime DESC LIMIT 500"
            ))
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![like], map_file)
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| e.to_string())?);
        }
        Ok(out)
    }

    /// ADD.md §6.1 tag search: exact (NOCASE) -> contains LIKE -> empty.
    pub fn tag_search(&self, query: &str) -> Result<Vec<FileRecord>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let sql = format!("SELECT {FILE_COLS_F} FROM files f JOIN file_tags ft ON ft.file_id = f.id JOIN tags t ON t.id = ft.tag_id WHERE t.name = ?1 COLLATE NOCASE ORDER BY f.mtime DESC LIMIT 500");
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let rows: Vec<FileRecord> = stmt
            .query_map(params![query], map_file)
            .map_err(|e| e.to_string())?
            .collect::<SqliteResult<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        if !rows.is_empty() {
            return Ok(rows);
        }
        let like = format!("%{}%", query);
        let sql2 = format!("SELECT {FILE_COLS_F} FROM files f JOIN file_tags ft ON ft.file_id = f.id JOIN tags t ON t.id = ft.tag_id WHERE t.name LIKE ?1 ORDER BY f.mtime DESC LIMIT 500");
        let mut stmt2 = conn.prepare(&sql2).map_err(|e| e.to_string())?;
        let rows2: Vec<FileRecord> = stmt2
            .query_map(params![like], map_file)
            .map_err(|e| e.to_string())?
            .collect::<SqliteResult<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        Ok(rows2)
    }

    /// Upsert tags + file_tags associations (source: 0=manual, 1=AI).
    pub fn set_file_tags(&self, file_id: i64, tags: &[(String, f32)], source: i64) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        for (name, conf) in tags {
            conn.execute(
                "INSERT OR IGNORE INTO tags(name) VALUES(?1)",
                params![name],
            )
            .map_err(|e| e.to_string())?;
            let tag_id: i64 = conn
                .query_row("SELECT id FROM tags WHERE name=?1", params![name], |r| r.get(0))
                .map_err(|e| e.to_string())?;
            conn.execute(
                "INSERT OR REPLACE INTO file_tags(file_id, tag_id, confidence, source) VALUES(?1, ?2, ?3, ?4)",
                params![file_id, tag_id, *conf as f64, source],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// (tag name, confidence, source) for one file, best confidence first.
    pub fn get_file_tags(&self, file_id: i64) -> Result<Vec<FileTag>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT t.name, ft.confidence, ft.source FROM file_tags ft JOIN tags t ON t.id = ft.tag_id WHERE ft.file_id = ?1 ORDER BY ft.confidence DESC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![file_id], |r| {
                Ok(FileTag {
                    name: r.get(0)?,
                    confidence: r.get(1)?,
                    source: r.get(2)?,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| e.to_string())?);
        }
        Ok(out)
    }

    /// Replace the manual (source=0) tags of a file with the given names.
    /// Empty list clears them. The "unknown" auto tag is untouched unless a
    /// real tag is present — a manually tagged photo must not keep the
    /// "unknown" sentinel (C-13.1).
    pub fn replace_manual_tags(&self, file_id: i64, names: &[String]) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "DELETE FROM file_tags WHERE file_id=?1 AND source=0",
            params![file_id],
        )
        .map_err(|e| e.to_string())?;
        let mut added = 0usize;
        for name in names {
            let name = name.trim();
            if name.is_empty() {
                continue;
            }
            conn.execute(
                "INSERT OR IGNORE INTO tags(name) VALUES(?1)",
                params![name],
            )
            .map_err(|e| e.to_string())?;
            let tag_id: i64 = conn
                .query_row("SELECT id FROM tags WHERE name=?1", params![name], |r| r.get(0))
                .map_err(|e| e.to_string())?;
            conn.execute(
                "INSERT OR REPLACE INTO file_tags(file_id, tag_id, confidence, source) VALUES(?1, ?2, 1.0, 0)",
                params![file_id, tag_id],
            )
            .map_err(|e| e.to_string())?;
            added += 1;
        }
        if added > 0 {
            conn.execute(
                "DELETE FROM file_tags WHERE file_id=?1 AND tag_id IN (SELECT id FROM tags WHERE name='unknown' COLLATE NOCASE)",
                params![file_id],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Batch-add MANUAL tags (source=0) to many files at once (multi-select
    /// "add tag" — APPEND semantics, existing tags are kept). Inserting a tag
    /// a file already carries is a no-op replace. Any "unknown" sentinel on
    /// the affected files is removed (C-13.1: manual tags displace it, same
    /// rule as an AI match). Returns the number of (file × tag) rows written.
    pub fn add_manual_tags_batch(
        &self,
        ids: &[i64],
        names: &[String],
    ) -> Result<usize, String> {
        if ids.is_empty() || names.is_empty() {
            return Ok(0);
        }
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut written = 0usize;
        for raw in names {
            let name = raw.trim();
            if name.is_empty() {
                continue;
            }
            conn.execute(
                "INSERT OR IGNORE INTO tags(name) VALUES(?1)",
                params![name],
            )
            .map_err(|e| e.to_string())?;
            let tag_id: i64 = conn
                .query_row("SELECT id FROM tags WHERE name=?1", params![name], |r| r.get(0))
                .map_err(|e| e.to_string())?;
            for &fid in ids {
                conn.execute(
                    "INSERT OR REPLACE INTO file_tags(file_id, tag_id, confidence, source) VALUES(?1, ?2, 1.0, 0)",
                    params![fid, tag_id],
                )
                .map_err(|e| e.to_string())?;
                written += 1;
            }
        }
        // Manual tags displace the "unknown" sentinel (C-13.1) — a photo that
        // carries at least one real tag must not keep it.
        let ph = vec!["?"; ids.len()].join(",");
        conn.execute(
            &format!(
                "DELETE FROM file_tags WHERE tag_id IN (SELECT id FROM tags WHERE name='unknown' COLLATE NOCASE) AND file_id IN ({ph})"
            ),
            rusqlite::params_from_iter(ids.iter()),
        )
        .map_err(|e| e.to_string())?;
        Ok(written)
    }

    /// Color labels (C-14): apply/remove one color for a set of files with
    /// phone-gallery semantics — if EVERY selected file already carries the
    /// color it is removed from all of them; otherwise it is added to the
    /// files that lack it (idempotent). Returns whether all files carry the
    /// color after the operation.
    pub fn toggle_color_tag(&self, ids: &[i64], color: &str) -> Result<bool, String> {
        if ids.is_empty() {
            return Ok(false);
        }
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let ph = vec!["?"; ids.len()].join(",");
        let mut params: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(ids.len() + 1);
        params.push(&color);
        for id in ids {
            params.push(id);
        }
        let mut stmt = conn
            .prepare(&format!(
                "SELECT COUNT(*) FROM color_tags WHERE color = ?1 AND file_id IN ({ph})"
            ))
            .map_err(|e| e.to_string())?;
        let have: i64 = stmt
            .query_row(params.as_slice(), |r| r.get(0))
            .map_err(|e| e.to_string())?;
        if have as usize == ids.len() {
            // Every selected file carries it → remove from all.
            conn.execute(
                &format!("DELETE FROM color_tags WHERE color = ?1 AND file_id IN ({ph})"),
                params.as_slice(),
            )
            .map_err(|e| e.to_string())?;
            Ok(false)
        } else {
            // Add to every file (INSERT OR IGNORE — idempotent).
            for &fid in ids {
                conn.execute(
                    "INSERT OR IGNORE INTO color_tags(file_id, color) VALUES(?1, ?2)",
                    params![fid, color],
                )
                .map_err(|e| e.to_string())?;
            }
            Ok(true)
        }
    }

    /// Every tag name the user could add: tags ever applied (manual or AI)
    /// PLUS user-defined custom tags that are not applied anywhere yet
    /// (C-13 fix: add_custom_tag only writes custom_tags, so an unused
    /// custom tag never appears in `tags` and must be merged in). Sorted by
    /// most-used first. The "unknown" sentinel is excluded.
    pub fn get_all_tag_names(&self) -> Result<Vec<String>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT name, MAX(c) FROM (
                   SELECT t.name AS name, COUNT(DISTINCT ft.file_id) AS c
                   FROM tags t LEFT JOIN file_tags ft ON ft.tag_id = t.id
                   WHERE lower(t.name) != 'unknown'
                   GROUP BY t.name
                   UNION ALL
                   SELECT ct.name AS name, 0 AS c FROM custom_tags ct
                   WHERE lower(ct.name) != 'unknown'
                 )
                 GROUP BY name ORDER BY MAX(c) DESC, name",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| e.to_string())?);
        }
        Ok(out)
    }

    /// Whether a file already has a stored SigLIP embedding (avoids
    /// re-embedding legacy files that only need tagging).
    pub fn has_embedding(&self, id: i64) -> Result<bool, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let has: Option<i64> = conn
            .query_row(
                "SELECT 1 FROM files WHERE id=?1 AND embedding IS NOT NULL",
                params![id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        Ok(has.is_some())
    }

    /// Load a stored embedding (f32 LE blob) — avoids re-running the vision
    /// encoder when re-tagging files that already have one.
    pub fn get_embedding(&self, id: i64) -> Result<Option<Vec<f32>>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let bytes: Option<Vec<u8>> = conn
            .query_row(
                "SELECT embedding FROM files WHERE id=?1",
                params![id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        Ok(bytes.map(|b| {
            b.chunks_exact(4)
                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect()
        }))
    }

    /// One-time re-index (C-09): embeddings written by older builds (raw
    /// last_hidden_state pooling) live in a DIFFERENT space than the
    /// pooler_output embeddings — mixing them breaks similarity. Wipe
    /// embeddings + auto tags (source=1) so the queue recomputes everything;
    /// manual tags are kept.
    pub fn reindex_embeddings(&self) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute_batch(
            "UPDATE files SET embedding = NULL, ai_processed = 0;
             DELETE FROM file_tags WHERE source = 1;",
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Files missing AT LEAST ONE currently-defined custom tag (C-12) — the
    /// target set of the manual "AI Tagging" pass. Includes never-indexed
    /// files (any ai_processed level): the TagAll task embeds them first.
    /// Paged via (limit, offset) so large libraries can be enqueued in
    /// batches. A manual tag with the same name as a custom tag counts as
    /// covering it (COLLATE NOCASE), same semantics as the old per-tag query.
    pub fn get_files_missing_any_tag(
        &self,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<(i64, String)>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT f.id, COALESCE(f.display_path, f.path) FROM files f
                 WHERE EXISTS (
                   SELECT 1 FROM custom_tags ct
                   WHERE NOT EXISTS (
                     SELECT 1 FROM file_tags ft JOIN tags t ON t.id = ft.tag_id
                     WHERE ft.file_id = f.id AND t.name = ct.name COLLATE NOCASE
                   )
                 )
                 ORDER BY f.id LIMIT ?1 OFFSET ?2",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![limit, offset], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| e.to_string())?);
        }
        Ok(out)
    }

    /// Drop the "unknown" sentinel tag of a file (before re-tagging runs).
    pub fn clear_unknown_tag(&self, file_id: i64) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "DELETE FROM file_tags WHERE file_id=?1 AND tag_id IN (SELECT id FROM tags WHERE name='unknown' COLLATE NOCASE)",
            params![file_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Self-heal: remove the "unknown" sentinel from any photo that also
    /// carries a real tag (an older bug could leave both). Runs at startup.
    pub fn cleanup_stray_unknown(&self) -> Result<usize, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let n = conn
            .execute(
                "DELETE FROM file_tags WHERE tag_id IN (SELECT id FROM tags WHERE name='unknown' COLLATE NOCASE)
                 AND file_id IN (
                   SELECT file_id FROM file_tags WHERE tag_id NOT IN (SELECT id FROM tags WHERE name='unknown' COLLATE NOCASE)
                 )",
                [],
            )
            .map_err(|e| e.to_string())?;
        Ok(n)
    }

    /// Set a free-text description for a file (used by the description search box)
    pub fn update_description(&self, id: i64, description: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE files SET description=?1 WHERE id=?2",
            params![description, id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Upsert a scanned file. Returns the file id. Resets ai_processed on
    /// insert or content change so the AI queue re-processes it.
    pub fn upsert_file(
        &self,
        folder_id: i64,
        path_key: &str,
        display_path: &str,
        filename: &str,
        size: i64,
        mtime: i64,
    ) -> Result<i64, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            r#"INSERT INTO files(folder_id, path, display_path, filename, size, mtime, created_at, ai_processed)
               VALUES(?1,?2,?3,?4,?5,?6,?7,0)
               ON CONFLICT(path) DO UPDATE SET folder_id=excluded.folder_id, display_path=excluded.display_path,
                 filename=excluded.filename, size=excluded.size, mtime=excluded.mtime, ai_processed=0,
                 exif_checked=0"#,
            params![folder_id, path_key, display_path, filename, size, mtime, now],
        )
        .map_err(|e| e.to_string())?;
        let id: i64 = conn
            .query_row(
                "SELECT id FROM files WHERE path=?1",
                params![path_key],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        Ok(id)
    }

    /// Delete files that no longer exist; returns deleted count
    pub fn delete_missing(
        &self,
        folder_id: i64,
        existing_keys: &std::collections::HashSet<String>,
    ) -> Result<usize, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT path FROM files WHERE folder_id=?1")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![folder_id], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        let mut to_delete = Vec::new();
        for r in rows {
            let p = r.map_err(|e| e.to_string())?;
            if !existing_keys.contains(&p) {
                to_delete.push(p);
            }
        }
        drop(stmt);
        let mut deleted = 0;
        for p in to_delete {
            conn.execute("DELETE FROM files WHERE path=?1", params![p])
                .map_err(|e| e.to_string())?;
            deleted += 1;
        }
        Ok(deleted)
    }

    /// path_key -> (size, mtime) for the given folder (incremental scan).
    pub fn get_file_map(
        &self,
        folder_id: i64,
    ) -> Result<std::collections::HashMap<String, (i64, i64)>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT path, size, mtime FROM files WHERE folder_id=?1")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![folder_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        let mut map = std::collections::HashMap::new();
        for r in rows {
            let (p, s, m) = r.map_err(|e| e.to_string())?;
            map.insert(p, (s, m));
        }
        Ok(map)
    }

    // ---- AI state ----

    /// Files waiting for AI processing (ai_processed = 0), oldest first.
    pub fn get_pending_ai_files(&self, limit: i64) -> Result<Vec<(i64, String)>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT id, COALESCE(display_path, path) FROM files WHERE ai_processed = 0 ORDER BY id LIMIT ?1",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![limit], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| e.to_string())?);
        }
        Ok(out)
    }

    pub fn set_ai_processed(&self, id: i64, level: i64) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE files SET ai_processed=?1 WHERE id=?2",
            params![level, id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn update_embedding(&self, id: i64, vec: &[f32]) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let bytes: Vec<u8> = vec
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        conn.execute(
            "UPDATE files SET embedding=?1 WHERE id=?2",
            params![bytes, id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Store EXIF lens/focal for a file (C-15) and mark it exif-checked so
    /// the startup backfill never re-reads it (files without EXIF stay NULL).
    pub fn update_exif(
        &self,
        id: i64,
        lens: Option<&str>,
        focal_length: Option<f64>,
    ) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE files SET lens=?1, focal_length=?2, exif_checked=1 WHERE id=?3",
            params![lens, focal_length, id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Files whose EXIF hasn't been read yet (C-15 startup backfill), paged.
    pub fn get_files_missing_exif(
        &self,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<(i64, String)>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT id, COALESCE(display_path, path) FROM files WHERE exif_checked = 0 ORDER BY id LIMIT ?1 OFFSET ?2",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![limit, offset], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| e.to_string())?);
        }
        Ok(out)
    }

    /// Distinct lens names across the library (C-15 filter panel), sorted.
    /// The "----" placeholder (cameras report "no lens mounted") is skipped.
    pub fn get_lens_list(&self) -> Result<Vec<String>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT DISTINCT lens FROM files WHERE lens IS NOT NULL AND lens != '' AND lens != '----' ORDER BY lens",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| e.to_string())?);
        }
        Ok(out)
    }

    /// Remove EVERY tag (text tags of any source AND color labels) from the
    /// given files — the multi-select "delete tags" action (C-15.1).
    pub fn clear_all_tags_on_files(&self, ids: &[i64]) -> Result<usize, String> {
        if ids.is_empty() {
            return Ok(0);
        }
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let ph = vec!["?"; ids.len()].join(",");
        let mut params: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(ids.len());
        for id in ids {
            params.push(id);
        }
        let removed = conn
            .execute(
                &format!("DELETE FROM file_tags WHERE file_id IN ({ph})"),
                params.as_slice(),
            )
            .map_err(|e| e.to_string())?
            + conn
                .execute(
                    &format!("DELETE FROM color_tags WHERE file_id IN ({ph})"),
                    params.as_slice(),
                )
                .map_err(|e| e.to_string())?;
        Ok(removed)
    }

    /// (id, embedding) for every file with an embedding (ai_processed >= 2).
    pub fn get_embeddings(&self) -> Result<Vec<(i64, Vec<f32>)>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT id, embedding FROM files WHERE ai_processed >= 2 AND embedding IS NOT NULL")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?)))
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for r in rows {
            let (id, bytes) = r.map_err(|e| e.to_string())?;
            let vec: Vec<f32> = bytes
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect();
            out.push((id, vec));
        }
        Ok(out)
    }

    /// Fetch file records by ids, preserving the given order.
    pub fn get_files_by_ids(&self, ids: &[i64]) -> Result<Vec<FileRecord>, String> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let placeholders = vec!["?"; ids.len()].join(",");
        let sql = format!(
            "SELECT {FILE_COLS} FROM files WHERE id IN ({placeholders})"
        );
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let params: Vec<&dyn rusqlite::ToSql> = ids.iter().map(|i| i as &dyn rusqlite::ToSql).collect();
        let rows = stmt
            .query_map(params.as_slice(), map_file)
            .map_err(|e| e.to_string())?;
        let mut by_id: std::collections::HashMap<i64, FileRecord> = std::collections::HashMap::new();
        for r in rows {
            let rec = r.map_err(|e| e.to_string())?;
            by_id.insert(rec.id, rec);
        }
        Ok(ids.iter().filter_map(|id| by_id.get(id).cloned()).collect())
    }

    // ---- custom tags (ADD.md §2) ----

    pub fn add_custom_tag(&self, name: &str, embedding: &[f32], threshold: f64) -> Result<i64, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let bytes: Vec<u8> = embedding.iter().flat_map(|v| v.to_le_bytes()).collect();
        conn.execute(
            "INSERT OR IGNORE INTO custom_tags(name, embedding, threshold) VALUES(?1, ?2, ?3)",
            params![name, bytes, threshold],
        )
        .map_err(|e| e.to_string())?;
        let id: i64 = conn
            .query_row("SELECT id FROM custom_tags WHERE name=?1", params![name], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        Ok(id)
    }

    pub fn delete_custom_tag(&self, id: i64) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM custom_tags WHERE id=?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Name of a custom tag (needed before deletion so its tag rows can be
    /// removed from every photo too).
    pub fn get_custom_tag_name(&self, id: i64) -> Result<Option<String>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.query_row(
            "SELECT name FROM custom_tags WHERE id=?1",
            params![id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())
    }

    /// Remove a tag name everywhere: the tags row deletion cascades to all
    /// file_tags associations (FK ON DELETE CASCADE) — cards, tag search
    /// and counts all update at once.
    pub fn remove_tag_everywhere(&self, name: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "DELETE FROM tags WHERE name=?1 COLLATE NOCASE",
            params![name],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Full tag reset (settings "clear tags"): removes every tag definition
    /// (custom_tags) and every tag assignment (file_tags + tags table).
    pub fn clear_all_tags(&self) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute_batch("DELETE FROM file_tags; DELETE FROM tags; DELETE FROM custom_tags;")
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn get_custom_tags(&self) -> Result<Vec<CustomTag>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT ct.id, ct.name, ct.threshold, ct.ref_count, ct.enabled, \
                 (SELECT COUNT(DISTINCT ft.file_id) FROM file_tags ft JOIN tags t ON t.id = ft.tag_id WHERE t.name = ct.name COLLATE NOCASE) \
                 FROM custom_tags ct ORDER BY ct.id",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok(CustomTag {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    threshold: r.get(2)?,
                    ref_count: r.get(3)?,
                    enabled: r.get(4)?,
                    photo_count: r.get(5)?,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| e.to_string())?);
        }
        Ok(out)
    }

    /// Get a persisted key/value setting (e.g. UI language)
    pub fn get_setting(&self, key: &str) -> Result<Option<String>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let val = conn
            .query_row(
                "SELECT value FROM settings WHERE key=?1",
                params![key],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        Ok(val)
    }

    /// Persist a key/value setting (upsert)
    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO settings(key, value) VALUES(?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, value],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }
}

fn map_file(row: &rusqlite::Row) -> SqliteResult<FileRecord> {
    let tags_raw: Option<String> = row.get(9)?;
    let colors_raw: Option<String> = row.get(10)?;
    let lens: Option<String> = row.get(11)?;
    let focal_length: Option<f64> = row.get(12)?;
    let tags: Vec<String> = tags_raw
        .map(|s| s.split(',').map(|t| t.to_string()).collect())
        .unwrap_or_default();
    let colors: Vec<String> = colors_raw
        .map(|s| s.split(',').map(|t| t.to_string()).collect())
        .unwrap_or_default();
    Ok(FileRecord {
        id: row.get(0)?,
        folder_id: row.get(1)?,
        path: row.get(2)?,
        filename: row.get(3)?,
        size: row.get(4)?,
        mtime: row.get(5)?,
        created_at: row.get(6)?,
        description: row.get(7)?,
        ai_processed: row.get(8)?,
        score: None,
        tags,
        colors,
        lens,
        focal_length,
    })
}
// Unit tests: path normalization, DB migration, scanner, tag search (ADD.md §11).

#[cfg(test)]
mod tests {
    use crate::db::Db;
    use crate::scanner;
    use crate::utils::normalize_storage_path;

    fn tmp_db() -> (tempfile::TempDir, Db) {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::new(dir.path().join("test.sqlite")).unwrap();
        (dir, db)
    }

    #[test]
    fn normalize_path_lowercases_and_uses_forward_slashes() {
        let key = normalize_storage_path("C:\\Users\\Test\\Photos\\DSC001.JPG");
        assert_eq!(key, "c:/users/test/photos/dsc001.jpg");
        let key2 = normalize_storage_path("c:/users/test/Photos/");
        assert_eq!(key2, "c:/users/test/photos");
    }

    #[test]
    fn migrate_adds_new_columns() {
        // Create an old-schema DB by hand, then open with Db::new and verify.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.sqlite");
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE folders (id INTEGER PRIMARY KEY AUTOINCREMENT, path TEXT UNIQUE NOT NULL, created_at INTEGER NOT NULL);
                 CREATE TABLE files (id INTEGER PRIMARY KEY AUTOINCREMENT, folder_id INTEGER NOT NULL, path TEXT UNIQUE NOT NULL, filename TEXT NOT NULL, size INTEGER NOT NULL, mtime INTEGER NOT NULL, created_at INTEGER NOT NULL, description TEXT NOT NULL DEFAULT '');
                 CREATE TABLE tags (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT UNIQUE NOT NULL);
                 CREATE TABLE file_tags (file_id INTEGER NOT NULL, tag_id INTEGER NOT NULL, PRIMARY KEY(file_id, tag_id));
                 CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 INSERT INTO folders(path, created_at) VALUES('E:/Photos', 1);
                 INSERT INTO files(folder_id, path, filename, size, mtime, created_at) VALUES(1, 'E:/Photos/DSC001.JPG', 'DSC001.JPG', 10, 20, 30);",
            )
            .unwrap();
        }
        let db = Db::new(&path).unwrap();
        let folders = db.get_folders().unwrap();
        assert_eq!(folders.len(), 1);
        // storage key lowercased, display preserved
        assert_eq!(folders[0].path, "E:/Photos");
        let photos = db.get_photos(None).unwrap();
        assert_eq!(photos.len(), 1);
        assert_eq!(photos[0].path, "E:/Photos/DSC001.JPG");
        assert_eq!(photos[0].ai_processed, 0);
    }

    #[test]
    fn scanner_detects_add_change_delete() {
        let (_dir, db) = tmp_db();
        let folder = tempfile::tempdir().unwrap();
        let fid = db.add_folder(folder.path().to_str().unwrap()).unwrap();

        // add
        std::fs::write(folder.path().join("a.jpg"), b"hello").unwrap();
        let (changed, deleted, pending) = scanner::scan_folder(&db, fid, folder.path().to_str().unwrap()).unwrap();
        assert_eq!(changed, 1);
        assert_eq!(deleted, 0);
        assert_eq!(pending.len(), 1);

        // no change
        let (changed, _, _) = scanner::scan_folder(&db, fid, folder.path().to_str().unwrap()).unwrap();
        assert_eq!(changed, 0);

        // change (mtime/size)
        std::fs::write(folder.path().join("a.jpg"), b"hello world").unwrap();
        let (changed, _, pending) = scanner::scan_folder(&db, fid, folder.path().to_str().unwrap()).unwrap();
        assert_eq!(changed, 1);
        assert_eq!(pending.len(), 1);

        // delete
        std::fs::remove_file(folder.path().join("a.jpg")).unwrap();
        let (_, deleted, _) = scanner::scan_folder(&db, fid, folder.path().to_str().unwrap()).unwrap();
        assert_eq!(deleted, 1);
        assert_eq!(db.get_photos(Some(fid)).unwrap().len(), 0);
    }

    #[test]
    fn tag_search_exact_then_contains() {
        let (_dir, db) = tmp_db();
        let folder = tempfile::tempdir().unwrap();
        let fid = db.add_folder(folder.path().to_str().unwrap()).unwrap();
        let p1 = folder.path().join("one.jpg");
        let p2 = folder.path().join("two.jpg");
        std::fs::write(&p1, b"x").unwrap();
        std::fs::write(&p2, b"y").unwrap();
        let _ = scanner::scan_folder(&db, fid, folder.path().to_str().unwrap()).unwrap();
        let files = db.get_photos(Some(fid)).unwrap();
        // manual tags
        db.set_file_tags(files[0].id, &[("cat".to_string(), 0.9)], 1).unwrap();
        db.set_file_tags(files[1].id, &[("catgirl".to_string(), 0.8)], 1).unwrap();

        let exact = db.tag_search("CAT").unwrap(); // NOCASE exact
        assert_eq!(exact.len(), 1);
        // Exact hit returns immediately (ADD.md §6.1); "cat" matches "cat" only.
        let exact2 = db.tag_search("cat").unwrap();
        assert_eq!(exact2.len(), 1);
        // Contains is the fallback when exact finds nothing.
        let contains = db.tag_search("catg").unwrap();
        assert_eq!(contains.len(), 1);
        assert_eq!(contains[0].filename, "two.jpg");
        let none = db.tag_search("zzz").unwrap();
        assert!(none.is_empty());
    }

    #[test]
    fn custom_tags_crud() {
        let (_dir, db) = tmp_db();
        let id = db.add_custom_tag("mytag", &[0.1, 0.2, 0.3], 0.3).unwrap();
        let tags = db.get_custom_tags().unwrap();
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].name, "mytag");
        db.delete_custom_tag(id).unwrap();
        assert!(db.get_custom_tags().unwrap().is_empty());
    }

    #[test]
    fn files_missing_any_tag_semantics() {
        let (_dir, db) = tmp_db();
        let folder = tempfile::tempdir().unwrap();
        let fid = db.add_folder(folder.path().to_str().unwrap()).unwrap();
        std::fs::write(folder.path().join("a.jpg"), b"x").unwrap();
        std::fs::write(folder.path().join("b.jpg"), b"y").unwrap();
        let _ = scanner::scan_folder(&db, fid, folder.path().to_str().unwrap()).unwrap();
        let files = db.get_photos(Some(fid)).unwrap();
        assert_eq!(files.len(), 2);

        // No tags defined -> nothing is "missing" (the pass requires tags).
        assert!(db.get_files_missing_any_tag(100, 0).unwrap().is_empty());

        // One tag, only file a tagged -> b is missing it.
        db.add_custom_tag("cat", &[0.1, 0.2, 0.3], 0.3).unwrap();
        db.set_file_tags(files[0].id, &[("cat".to_string(), 0.9)], 1).unwrap();
        let missing = db.get_files_missing_any_tag(100, 0).unwrap();
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].0, files[1].id);

        // Tag both -> nothing missing.
        db.set_file_tags(files[1].id, &[("cat".to_string(), 0.9)], 1).unwrap();
        assert!(db.get_files_missing_any_tag(100, 0).unwrap().is_empty());

        // Second tag added -> files carrying only the first are missing it
        // again (multi-label: a file can match several tags).
        db.add_custom_tag("dog", &[0.1, 0.2, 0.3], 0.3).unwrap();
        let missing = db.get_files_missing_any_tag(100, 0).unwrap();
        assert_eq!(missing.len(), 2);

        // Pagination: limit 1 skips one.
        let page = db.get_files_missing_any_tag(1, 0).unwrap();
        assert_eq!(page.len(), 1);
        let page2 = db.get_files_missing_any_tag(1, 1).unwrap();
        assert_eq!(page2.len(), 1);
        assert_ne!(page[0].0, page2[0].0);

        // A MANUAL tag with the same name covers the custom tag too.
        db.set_file_tags(files[0].id, &[("dog".to_string(), 1.0)], 0).unwrap();
        let missing = db.get_files_missing_any_tag(100, 0).unwrap();
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].0, files[1].id);
    }

    #[test]
    fn manual_tags_batch_and_name_list() {
        let (_dir, db) = tmp_db();
        let folder = tempfile::tempdir().unwrap();
        let fid = db.add_folder(folder.path().to_str().unwrap()).unwrap();
        std::fs::write(folder.path().join("a.jpg"), b"x").unwrap();
        std::fs::write(folder.path().join("b.jpg"), b"y").unwrap();
        let _ = scanner::scan_folder(&db, fid, folder.path().to_str().unwrap()).unwrap();
        let files = db.get_photos(Some(fid)).unwrap();
        let ids: Vec<i64> = files.iter().map(|f| f.id).collect();

        // Batch-add two tags to both files (append semantics).
        let n = db
            .add_manual_tags_batch(&ids, &["cat".to_string(), "dog".to_string()])
            .unwrap();
        assert_eq!(n, 4);
        for f in &files {
            let tags = db.get_file_tags(f.id).unwrap();
            assert_eq!(tags.len(), 2);
            assert!(tags.iter().all(|t| t.source == 0));
        }
        // Idempotent re-add: no duplicate rows.
        let n2 = db
            .add_manual_tags_batch(&ids, &["cat".to_string()])
            .unwrap();
        assert_eq!(n2, 2);
        assert_eq!(db.get_file_tags(files[0].id).unwrap().len(), 2);

        // Manual tags displace the "unknown" sentinel (C-13.1): a photo with
        // unknown that gets manually tagged loses it.
        db.set_file_tags(files[0].id, &[("unknown".to_string(), 1.0)], 1).unwrap();
        db.add_manual_tags_batch(&[files[0].id], &["fish".to_string()]).unwrap();
        let tags = db.get_file_tags(files[0].id).unwrap();
        assert!(tags.iter().all(|t| t.name != "unknown"));
        // Same via the edit-dialog path (replace_manual_tags).
        db.set_file_tags(files[0].id, &[("unknown".to_string(), 1.0)], 1).unwrap();
        db.replace_manual_tags(files[0].id, &["bird".to_string()]).unwrap();
        let tags = db.get_file_tags(files[0].id).unwrap();
        assert!(tags.iter().all(|t| t.name != "unknown"));
        // Clearing ALL manual tags keeps the sentinel (nothing real left).
        db.set_file_tags(files[0].id, &[("unknown".to_string(), 1.0)], 1).unwrap();
        db.replace_manual_tags(files[0].id, &[]).unwrap();
        let tags = db.get_file_tags(files[0].id).unwrap();
        assert!(tags.iter().any(|t| t.name == "unknown"));

        // Name list: most-used first, "unknown" sentinel excluded, and
        // custom tags that were never applied still show up (C-13 fix).
        db.set_file_tags(files[0].id, &[("unknown".to_string(), 1.0)], 1).unwrap();
        db.set_file_tags(files[0].id, &[("cat".to_string(), 0.9)], 1).unwrap(); // reuses the cat row
        db.add_custom_tag("test", &[0.1, 0.2, 0.3], 0.3).unwrap(); // defined, applied nowhere
        let names = db.get_all_tag_names().unwrap();
        assert!(names.contains(&"cat".to_string()));
        assert!(names.contains(&"dog".to_string()));
        assert!(names.contains(&"test".to_string()));
        assert!(!names.iter().any(|n| n == "unknown"));
        assert_eq!(names[0], "cat"); // cat is on both files (2 uses), dog on 1, test on 0
    }

    #[test]
    fn color_tags_toggle() {
        let (_dir, db) = tmp_db();
        let folder = tempfile::tempdir().unwrap();
        let fid = db.add_folder(folder.path().to_str().unwrap()).unwrap();
        std::fs::write(folder.path().join("a.jpg"), b"x").unwrap();
        std::fs::write(folder.path().join("b.jpg"), b"y").unwrap();
        let _ = scanner::scan_folder(&db, fid, folder.path().to_str().unwrap()).unwrap();
        let files = db.get_photos(Some(fid)).unwrap();
        let ids: Vec<i64> = files.iter().map(|f| f.id).collect();

        // Nothing has red → toggle adds it to all.
        assert!(db.toggle_color_tag(&ids, "red").unwrap());
        // All have red now → toggle removes it from all.
        assert!(!db.toggle_color_tag(&ids, "red").unwrap());
        // A file can carry several colors at once.
        db.toggle_color_tag(&[ids[0]], "red").unwrap();
        db.toggle_color_tag(&[ids[0]], "blue").unwrap();
        let rec = db.get_file_by_id(ids[0]).unwrap().unwrap();
        assert!(rec.colors.contains(&"red".to_string()));
        assert!(rec.colors.contains(&"blue".to_string()));
        assert!(rec.tags.is_empty());
        // Mixed: only file0 has green → toggle adds to file1 too (all=true).
        assert!(db.toggle_color_tag(&ids, "green").unwrap());
        let rec1 = db.get_file_by_id(ids[1]).unwrap().unwrap();
        assert!(rec1.colors.contains(&"green".to_string()));
        // Deleting the files cascades their color rows (foreign_keys=ON).
        db.delete_missing(fid, &std::collections::HashSet::new()).unwrap();
        assert!(db.get_photos(Some(fid)).unwrap().is_empty());
    }

    #[test]
    fn exif_columns_roundtrip() {
        let (_dir, db) = tmp_db();
        let folder = tempfile::tempdir().unwrap();
        let fid = db.add_folder(folder.path().to_str().unwrap()).unwrap();
        std::fs::write(folder.path().join("a.jpg"), b"x").unwrap();
        let _ = scanner::scan_folder(&db, fid, folder.path().to_str().unwrap()).unwrap();
        let id = db.get_photos(Some(fid)).unwrap()[0].id;

        // The scanner already marked the file exif-checked (no EXIF found).
        assert!(db.get_files_missing_exif(100, 0).unwrap().is_empty());
        // Backfill-style update: explicit lens/focal wins.
        db.update_exif(id, Some("EF50mm f/1.8"), Some(50.0)).unwrap();
        let rec = db.get_file_by_id(id).unwrap().unwrap();
        assert_eq!(rec.lens.as_deref(), Some("EF50mm f/1.8"));
        assert_eq!(rec.focal_length, Some(50.0));
        assert_eq!(db.get_lens_list().unwrap(), vec!["EF50mm f/1.8".to_string()]);
        // A changed file is re-marked for extraction — simulate a content
        // change via upsert directly (the scanner would re-extract right
        // away, so bypass it here).
        let display = folder.path().join("a.jpg").to_string_lossy().to_string();
        let key = crate::utils::normalize_storage_path(&display);
        db.upsert_file(fid, &key, &display, "a.jpg", 999, 999).unwrap();
        let missing = db.get_files_missing_exif(100, 0).unwrap();
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].0, id);
    }

    #[test]
    fn settings_roundtrip() {
        let (_dir, db) = tmp_db();
        db.set_setting("k", "v").unwrap();
        assert_eq!(db.get_setting("k").unwrap(), Some("v".to_string()));
    }
}

