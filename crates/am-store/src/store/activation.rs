use rusqlite::params;
use uuid::Uuid;

use am_core::ActivationStats;

use crate::error::{Result, StoreError};

use super::Store;

impl Store {
    pub fn increment_activation(&self, occurrence_id: Uuid) -> Result<()> {
        let rows = self.conn.execute(
            "UPDATE occurrences SET activation_count = activation_count + 1 WHERE id = ?1",
            [occurrence_id.to_string()],
        )?;
        if rows == 0 {
            return Err(StoreError::InvalidData(format!(
                "occurrence not found: {occurrence_id}"
            )));
        }
        Ok(())
    }

    /// Increment `activation_count` for multiple occurrences in a single transaction.
    ///
    /// Silently skips IDs that do not exist in the store (common when the
    /// system has occurrences that were never persisted, e.g. from conscious
    /// memory added after the last full save).
    pub fn batch_increment_activation(&self, ids: &[Uuid]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare(
                "UPDATE occurrences SET activation_count = activation_count + 1 WHERE id = ?1",
            )?;
            for id in ids {
                stmt.execute([id.to_string()])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Set activation counts to absolute values for a batch of occurrences.
    ///
    /// Used by feedback demote where activation is decremented rather than
    /// incremented. Silently skips unknown IDs (common for unpersisted
    /// conscious occurrences).
    pub fn batch_set_activation_counts(&self, batch: &[(Uuid, u32)]) -> Result<()> {
        if batch.is_empty() {
            return Ok(());
        }
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut stmt =
                tx.prepare("UPDATE occurrences SET activation_count = ?1 WHERE id = ?2")?;
            for (id, count) in batch {
                stmt.execute(rusqlite::params![count, id.to_string()])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Mark a neighborhood as superseded by another (targeted update, no full save).
    pub fn mark_superseded(&self, old_id: Uuid, new_id: Uuid) -> Result<()> {
        let rows = self.conn.execute(
            "UPDATE neighborhoods SET superseded_by = ?1 WHERE id = ?2",
            params![new_id.to_string(), old_id.to_string()],
        )?;
        if rows == 0 {
            return Err(StoreError::InvalidData(format!(
                "neighborhood not found: {old_id}"
            )));
        }
        Ok(())
    }

    /// Get activation count distribution for stats.
    pub fn activation_distribution(&self) -> Result<ActivationStats> {
        let total: u64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM occurrences", [], |row| row.get(0))?;
        let zero_activation: u64 = self.conn.query_row(
            "SELECT COUNT(*) FROM occurrences WHERE activation_count = 0",
            [],
            |row| row.get(0),
        )?;
        let max_activation: u32 = self.conn.query_row(
            "SELECT COALESCE(MAX(activation_count), 0) FROM occurrences",
            [],
            |row| row.get(0),
        )?;
        let sum_activation: u64 = self.conn.query_row(
            "SELECT COALESCE(SUM(activation_count), 0) FROM occurrences",
            [],
            |row| row.get(0),
        )?;

        Ok(ActivationStats {
            total,
            zero_activation,
            max_activation,
            mean_activation: if total > 0 {
                sum_activation as f64 / total as f64
            } else {
                0.0
            },
        })
    }

    // --- Conversation buffer ---

    pub fn append_buffer(&self, user_text: &str, assistant_text: &str) -> Result<usize> {
        self.conn.execute(
            "INSERT INTO conversation_buffer (user_text, assistant_text) VALUES (?1, ?2)",
            params![user_text, assistant_text],
        )?;
        let count: usize =
            self.conn
                .query_row("SELECT COUNT(*) FROM conversation_buffer", [], |row| {
                    row.get(0)
                })?;
        Ok(count)
    }

    pub fn drain_buffer(&self) -> Result<Vec<(String, String)>> {
        let tx = self.conn.unchecked_transaction()?;

        let mut stmt = tx
            .prepare("SELECT id, user_text, assistant_text FROM conversation_buffer ORDER BY id")?;
        let entries: Vec<(i64, String, String)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect::<std::result::Result<_, _>>()?;
        drop(stmt);

        if !entries.is_empty() {
            // Delete exactly the rows we read using a parameterized range
            // on the rowid. Since we fetched ORDER BY id, entries are
            // contiguous. Any rows arriving from another connection after
            // our SELECT will have id > max and survive for the next drain
            // (at-least-once semantics).
            let min_id = entries.first().expect("non-empty").0;
            let max_id = entries.last().expect("non-empty").0;
            tx.execute(
                "DELETE FROM conversation_buffer WHERE id >= ?1 AND id <= ?2",
                params![min_id, max_id],
            )?;
        }

        let results: Vec<(String, String)> = entries.into_iter().map(|(_, u, a)| (u, a)).collect();

        tx.commit()?;

        Ok(results)
    }

    pub fn buffer_count(&self) -> Result<usize> {
        let count: usize =
            self.conn
                .query_row("SELECT COUNT(*) FROM conversation_buffer", [], |row| {
                    row.get(0)
                })?;
        Ok(count)
    }
}
