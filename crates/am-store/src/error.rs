use std::fmt;

#[derive(Debug)]
pub enum StoreError {
    Sqlite(rusqlite::Error),
    Io(std::io::Error),
    InvalidData(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::Sqlite(e) => write!(f, "SQLite error: {e}"),
            StoreError::Io(e) => write!(f, "I/O error: {e}"),
            StoreError::InvalidData(msg) => write!(f, "invalid data: {msg}"),
        }
    }
}

impl std::error::Error for StoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            StoreError::Sqlite(e) => Some(e),
            StoreError::Io(e) => Some(e),
            StoreError::InvalidData(_) => None,
        }
    }
}

impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        StoreError::Sqlite(e)
    }
}

impl From<std::io::Error> for StoreError {
    fn from(e: std::io::Error) -> Self {
        StoreError::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, StoreError>;
