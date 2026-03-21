use crate::error::{Result, StoreError};

use super::{Store, parse_uuid};

impl Store {
    /// Delete a specific subconscious episode and all its contents.
    /// Returns the number of occurrences removed, or 0 if not found.
    pub fn forget_episode(&self, episode_id: &str) -> Result<u64> {
        let uuid = parse_uuid(episode_id)?;
        let id_str = uuid.to_string();

        // Verify it exists and is not conscious
        let is_conscious: Option<bool> = self
            .conn
            .query_row(
                "SELECT is_conscious FROM episodes WHERE id = ?1",
                [&id_str],
                |row| row.get(0),
            )
            .ok();

        match is_conscious {
            None => return Ok(0),
            Some(true) => {
                return Err(StoreError::InvalidData(
                    "use forget_conscious to remove conscious memories".into(),
                ));
            }
            Some(false) => {}
        }

        let tx = self.conn.unchecked_transaction()?;

        let removed: u64 = tx.execute(
            "DELETE FROM occurrences WHERE neighborhood_id IN (
                 SELECT id FROM neighborhoods WHERE episode_id = ?1
             )",
            [&id_str],
        )? as u64;

        tx.execute("DELETE FROM neighborhoods WHERE episode_id = ?1", [&id_str])?;

        tx.execute("DELETE FROM episodes WHERE id = ?1", [&id_str])?;

        tx.commit()?;
        Ok(removed)
    }

    /// Delete a specific conscious neighborhood by UUID.
    /// Returns the number of occurrences removed, or 0 if not found.
    pub fn forget_conscious(&self, neighborhood_id: &str) -> Result<u64> {
        let uuid = parse_uuid(neighborhood_id)?;
        let id_str = uuid.to_string();

        // Verify it's a conscious neighborhood
        let is_conscious: Option<bool> = self
            .conn
            .query_row(
                "SELECT e.is_conscious FROM neighborhoods n
                 JOIN episodes e ON n.episode_id = e.id
                 WHERE n.id = ?1",
                [&id_str],
                |row| row.get(0),
            )
            .ok();

        match is_conscious {
            None => return Ok(0),
            Some(false) => {
                return Err(StoreError::InvalidData(
                    "neighborhood is not conscious - use forget_episode instead".into(),
                ));
            }
            Some(true) => {}
        }

        let tx = self.conn.unchecked_transaction()?;

        let removed: u64 = tx.execute(
            "DELETE FROM occurrences WHERE neighborhood_id = ?1",
            [&id_str],
        )? as u64;

        tx.execute("DELETE FROM neighborhoods WHERE id = ?1", [&id_str])?;

        tx.commit()?;
        Ok(removed)
    }

    /// Delete all occurrences matching a word (case-insensitive), clean empty structures.
    /// Returns (removed_occurrences, removed_neighborhoods, removed_episodes).
    pub fn forget_term(&self, term: &str) -> Result<(u64, u64, u64)> {
        let word_lower = term.to_lowercase();

        let tx = self.conn.unchecked_transaction()?;

        let removed_occs: u64 = tx.execute(
            "DELETE FROM occurrences WHERE LOWER(word) = ?1",
            [&word_lower],
        )? as u64;

        // Clean empty neighborhoods (both conscious and subconscious)
        let removed_neighborhoods: u64 = tx.execute(
            "DELETE FROM neighborhoods WHERE id NOT IN (
                 SELECT DISTINCT neighborhood_id FROM occurrences
             )",
            [],
        )? as u64;

        // Clean empty non-conscious episodes
        let removed_episodes: u64 = tx.execute(
            "DELETE FROM episodes WHERE is_conscious = 0
             AND id NOT IN (
                 SELECT DISTINCT episode_id FROM neighborhoods
             )",
            [],
        )? as u64;

        tx.commit()?;
        Ok((removed_occs, removed_neighborhoods, removed_episodes))
    }
}
