mod activation;
mod core;
mod forget;
pub mod gc;
mod load;
mod persist;
mod query;

use rusqlite::Connection;
use uuid::Uuid;

use crate::error::{Result, StoreError};

#[derive(Debug)]
pub struct EpisodeInfo {
    pub id: String,
    pub name: String,
    pub is_conscious: bool,
    pub timestamp: String,
    pub neighborhood_count: u64,
    pub occurrence_count: u64,
    pub total_activation: u64,
}

#[derive(Debug)]
pub struct NeighborhoodInfo {
    pub id: String,
    pub source_text: String,
    pub occurrence_count: u64,
    pub total_activation: u64,
}

#[derive(Debug)]
pub struct NeighborhoodDetail {
    pub id: String,
    pub source_text: String,
    pub episode_name: String,
    pub is_conscious: bool,
    pub occurrence_count: u64,
    pub total_activation: u64,
    pub max_activation: u32,
}

pub struct Store {
    pub(crate) conn: Connection,
}

impl Drop for Store {
    fn drop(&mut self) {
        // Clean shutdown: flush WAL to main DB
        let _ = self.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
    }
}

pub(crate) fn parse_uuid(s: &str) -> Result<Uuid> {
    Uuid::parse_str(s).map_err(|e| StoreError::InvalidData(format!("invalid UUID '{s}': {e}")))
}

#[cfg(test)]
mod tests;
