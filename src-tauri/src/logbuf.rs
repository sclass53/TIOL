//! In-memory ring buffer of recent log lines, fed by env_logger's formatter.
//! Lets the settings debug-mode panel show app logs inside the UI
//! (get_logs command). Capped at 500 lines, no allocation churn beyond that.

use std::collections::VecDeque;
use std::sync::{OnceLock, RwLock};

const CAP: usize = 500;

static LOG_BUF: OnceLock<RwLock<VecDeque<String>>> = OnceLock::new();

/// Must be called once before env_logger initializes (so the formatter can push).
pub fn init() {
    let _ = LOG_BUF.set(RwLock::new(VecDeque::with_capacity(CAP)));
}

/// Append one formatted log line (oldest dropped when full).
pub fn push(line: String) {
    if let Some(buf) = LOG_BUF.get() {
        if let Ok(mut b) = buf.write() {
            if b.len() >= CAP {
                b.pop_front();
            }
            b.push_back(line);
        }
    }
}

/// Newest-first snapshot, at most `limit` lines.
pub fn snapshot(limit: usize) -> Vec<String> {
    match LOG_BUF.get() {
        Some(buf) => match buf.read() {
            Ok(b) => b.iter().rev().take(limit).cloned().collect(),
            Err(_) => Vec::new(),
        },
        None => Vec::new(),
    }
}
