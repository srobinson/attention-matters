use std::path::Path;

use rusqlite::{Connection, params};

use crate::error::{Result, StoreError};
use crate::schema;

use super::Store;

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        schema::initialize(&conn)?;
        Ok(Self { conn })
    }

    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        schema::initialize(&conn)?;
        Ok(Self { conn })
    }

    /// Verify the connection is still usable.
    pub fn health_check(&self) -> Result<()> {
        self.conn
            .execute_batch("SELECT 1")
            .map_err(StoreError::Sqlite)
    }

    /// Run a TRUNCATE checkpoint - flushes WAL and removes the file.
    /// Used during clean shutdown.
    pub fn checkpoint_truncate(&self) -> Result<()> {
        self.conn
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        Ok(())
    }

    // --- Metadata ---

    pub fn get_metadata(&self, key: &str) -> Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT value FROM metadata WHERE key = ?1")?;
        let result = stmt.query_row([key], |row| row.get(0)).ok();
        Ok(result)
    }

    pub fn set_metadata(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO metadata (key, value) VALUES (?1, ?2)",
            params![key, value],
        )?;
        Ok(())
    }

    /// Get the database file size in bytes (0 for in-memory databases).
    pub fn db_size(&self) -> u64 {
        let page_count: u64 = self
            .conn
            .query_row("PRAGMA page_count", [], |row| row.get(0))
            .unwrap_or(0);
        let page_size: u64 = self
            .conn
            .query_row("PRAGMA page_size", [], |row| row.get(0))
            .unwrap_or(4096);
        page_count * page_size
    }
}
