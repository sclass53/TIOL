//! Unified error type for TIOL (ADD.md §12: anyhow + thiserror).

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("database error: {0}")]
    Db(String),
    #[error("AI error: {0}")]
    Ai(String),
    #[error("model error: {0}")]
    Model(String),
    #[error("download error: {0}")]
    Download(String),
    #[error("search error: {0}")]
    Search(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self {
        AppError::Db(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, AppError>;
