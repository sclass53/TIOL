use rusqlite::{params, Connection, OptionalExtension, Result as SqliteResult};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Folder {
    pub id: i64,
    pub path: String,
    pub created_at: i64,
    pub photo_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRecord {
    pub id: i64,
    pub folder_id: i64,
    pub path: String,
    pub filename: String,
    pub size: i64,
    pub mtime: i64,
    pub created_at: i64,
    pub description: String,
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
        // Enable WAL for better concurrency, foreign keys
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
                created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS files (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                folder_id INTEGER NOT NULL,
                path TEXT UNIQUE NOT NULL,
                filename TEXT NOT NULL,
                size INTEGER NOT NULL,
                mtime INTEGER NOT NULL,
                created_at INTEGER NOT NULL,
                FOREIGN KEY(folder_id) REFERENCES folders(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_files_folder ON files(folder_id);
            CREATE INDEX IF NOT EXISTS idx_files_path ON files(path);
            CREATE INDEX IF NOT EXISTS idx_files_mtime ON files(mtime DESC);
            CREATE TABLE IF NOT EXISTS tags (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT UNIQUE NOT NULL
            );
            CREATE TABLE IF NOT EXISTS file_tags (
                file_id INTEGER NOT NULL,
                tag_id INTEGER NOT NULL,
                PRIMARY KEY(file_id, tag_id),
                FOREIGN KEY(file_id) REFERENCES files(id) ON DELETE CASCADE,
                FOREIGN KEY(tag_id) REFERENCES tags(id) ON DELETE CASCADE
            );
            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            "#,
        )?;
        // Migration: add description column for DBs created before this feature
        let cols: Vec<String> = conn
            .prepare("PRAGMA table_info(files)")?
            .query_map([], |r| r.get(1))?
            .collect::<SqliteResult<Vec<_>>>()?;
        if !cols.iter().any(|c| c == "description") {
            conn.execute_batch(
                "ALTER TABLE files ADD COLUMN description TEXT NOT NULL DEFAULT '';",
            )?;
        }
        Ok(())
    }

    pub fn add_folder(&self, path: &str) -> Result<i64, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let normalized = normalize_path(path);
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT OR IGNORE INTO folders(path, created_at) VALUES(?1, ?2)",
            params![normalized, now],
        )
        .map_err(|e| e.to_string())?;
        // return id
        let id: i64 = conn
            .query_row(
                "SELECT id FROM folders WHERE path=?1",
                params![normalized],
                |r| r.get(0),
            )
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
                r#"SELECT f.id, f.path, f.created_at, COUNT(fi.id) as cnt
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
                    photo_count: r.get(3)?,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| e.to_string())?);
        }
        Ok(out)
    }

    pub fn get_photos(&self, folder_id: Option<i64>) -> Result<Vec<FileRecord>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let (sql, param): (String, Option<i64>) = match folder_id {
            Some(fid) => (
                "SELECT id, folder_id, path, filename, size, mtime, created_at, description FROM files WHERE folder_id=?1 ORDER BY mtime DESC LIMIT 2000".to_string(),
                Some(fid),
            ),
            None => (
                "SELECT id, folder_id, path, filename, size, mtime, created_at, description FROM files ORDER BY mtime DESC LIMIT 2000".to_string(),
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

    pub fn search_files(&self, query: &str) -> Result<Vec<FileRecord>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let like = format!("%{}%", query);
        let mut stmt = conn
            .prepare(
                "SELECT id, folder_id, path, filename, size, mtime, created_at, description FROM files WHERE filename LIKE ?1 OR path LIKE ?1 ORDER BY mtime DESC LIMIT 500",
            )
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
            .prepare(
                "SELECT id, folder_id, path, filename, size, mtime, created_at, description FROM files WHERE description LIKE ?1 ORDER BY mtime DESC LIMIT 500",
            )
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

    pub fn upsert_file(
        &self,
        folder_id: i64,
        path: &str,
        filename: &str,
        size: i64,
        mtime: i64,
    ) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            r#"INSERT INTO files(folder_id, path, filename, size, mtime, created_at)
               VALUES(?1,?2,?3,?4,?5,?6)
               ON CONFLICT(path) DO UPDATE SET folder_id=excluded.folder_id, filename=excluded.filename, size=excluded.size, mtime=excluded.mtime"#,
            params![folder_id, path, filename, size, mtime, now],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Delete files that no longer exist; returns deleted count
    pub fn delete_missing(
        &self,
        folder_id: i64,
        existing_paths: &std::collections::HashSet<String>,
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
            if !existing_paths.contains(&p) {
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

    pub fn get_file_map(
        &self,
        folder_id: i64,
    ) -> Result<std::collections::HashMap<String, (i64, i64)>, String> {
        // path -> (size, mtime)
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
}

fn map_file(row: &rusqlite::Row) -> SqliteResult<FileRecord> {
    Ok(FileRecord {
        id: row.get(0)?,
        folder_id: row.get(1)?,
        path: row.get(2)?,
        filename: row.get(3)?,
        size: row.get(4)?,
        mtime: row.get(5)?,
        created_at: row.get(6)?,
        description: row.get(7)?,
    })
}

fn normalize_path(p: &str) -> String {
    // Cheap normalization: trim trailing slash, replace \ with /
    let mut s = p.trim().to_string();
    while s.ends_with('/') || s.ends_with('\\') {
        s.pop();
    }
    s
}
