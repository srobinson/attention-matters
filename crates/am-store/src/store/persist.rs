use rusqlite::{Connection, params};
use uuid::Uuid;

use am_core::{
    episode::Episode, neighborhood::Neighborhood, occurrence::Occurrence, phasor::DaemonPhasor,
    quaternion::Quaternion, system::DAESystem,
};

use crate::error::{Result, StoreError};

use super::Store;

impl Store {
    pub fn save_system(&self, system: &DAESystem) -> Result<()> {
        // Guard: refuse to overwrite existing data with an empty system.
        // This prevents data destruction when the server fails to load state
        // and then saves its empty in-memory system over the real data.
        if system.n() == 0 && system.episodes.is_empty() {
            let existing: i64 =
                self.conn
                    .query_row("SELECT COUNT(*) FROM occurrences", [], |r| r.get(0))?;
            if existing > 0 {
                return Err(StoreError::InvalidData(format!(
                    "refusing to overwrite {existing} existing occurrences with empty system \
                     (possible failed load)"
                )));
            }
        }

        let tx = self.conn.unchecked_transaction()?;

        // Clear existing data
        tx.execute_batch(
            "DELETE FROM occurrences; DELETE FROM neighborhoods; DELETE FROM episodes;",
        )?;

        self.set_metadata_on(&tx, "agent_name", &system.agent_name)?;

        // Save subconscious episodes
        for episode in &system.episodes {
            self.save_episode_on(&tx, episode)?;
        }

        // Save conscious episode
        self.save_episode_on(&tx, &system.conscious_episode)?;

        tx.commit()?;
        // PASSIVE checkpoint after bulk write - flushes WAL without blocking readers
        let _ = self.conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE);");
        Ok(())
    }

    fn set_metadata_on(&self, conn: &Connection, key: &str, value: &str) -> Result<()> {
        conn.execute(
            "INSERT OR REPLACE INTO metadata (key, value) VALUES (?1, ?2)",
            params![key, value],
        )?;
        Ok(())
    }

    /// Persist a single episode (and its neighborhoods/occurrences) without
    /// rewriting the entire system. Use after `DAESystem::add_episode` to
    /// avoid the full DELETE/rewrite cycle of `save_system`.
    pub fn save_episode(&self, episode: &Episode) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        self.save_episode_on(&tx, episode)?;
        tx.commit()?;
        Ok(())
    }

    /// Persist a single neighborhood under an episode without rewriting the
    /// entire system. Creates the episode row if it does not already exist
    /// (using INSERT OR IGNORE), then inserts the neighborhood and its
    /// occurrences. Use after adding a neighborhood to the conscious episode
    /// via `add_to_conscious` or `extract_salient`.
    pub fn save_neighborhood(&self, episode: &Episode, neighborhood: &Neighborhood) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        // Ensure the parent episode row exists (no-op if already present)
        tx.execute(
            "INSERT OR IGNORE INTO episodes (id, name, is_conscious, timestamp) VALUES (?1, ?2, ?3, ?4)",
            params![
                episode.id.to_string(),
                episode.name,
                episode.is_conscious as i32,
                episode.timestamp,
            ],
        )?;
        self.save_neighborhood_on(&tx, neighborhood, episode.id)?;
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn save_episode_on(&self, conn: &Connection, episode: &Episode) -> Result<()> {
        conn.execute(
            "INSERT INTO episodes (id, name, is_conscious, timestamp) VALUES (?1, ?2, ?3, ?4)",
            params![
                episode.id.to_string(),
                episode.name,
                episode.is_conscious as i32,
                episode.timestamp,
            ],
        )?;

        for neighborhood in &episode.neighborhoods {
            self.save_neighborhood_on(conn, neighborhood, episode.id)?;
        }

        Ok(())
    }

    fn save_neighborhood_on(
        &self,
        conn: &Connection,
        neighborhood: &Neighborhood,
        episode_id: Uuid,
    ) -> Result<()> {
        conn.execute(
            "INSERT INTO neighborhoods (id, episode_id, seed_w, seed_x, seed_y, seed_z, source_text, neighborhood_type, epoch, superseded_by)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                neighborhood.id.to_string(),
                episode_id.to_string(),
                neighborhood.seed.w,
                neighborhood.seed.x,
                neighborhood.seed.y,
                neighborhood.seed.z,
                neighborhood.source_text,
                neighborhood.neighborhood_type.as_str(),
                neighborhood.epoch,
                neighborhood.superseded_by.map(|id| id.to_string()),
            ],
        )?;

        for occurrence in &neighborhood.occurrences {
            self.save_occurrence_on(conn, occurrence)?;
        }

        Ok(())
    }

    fn save_occurrence_on(&self, conn: &Connection, occ: &Occurrence) -> Result<()> {
        conn.execute(
            "INSERT INTO occurrences (id, neighborhood_id, word, pos_w, pos_x, pos_y, pos_z, phasor_theta, activation_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                occ.id.to_string(),
                occ.neighborhood_id.to_string(),
                occ.word,
                occ.position.w,
                occ.position.x,
                occ.position.y,
                occ.position.z,
                occ.phasor.theta,
                occ.activation_count,
            ],
        )?;
        Ok(())
    }

    pub fn save_occurrence_positions(
        &self,
        batch: &[(Uuid, Quaternion, DaemonPhasor)],
    ) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare(
                "UPDATE occurrences SET pos_w = ?1, pos_x = ?2, pos_y = ?3, pos_z = ?4, phasor_theta = ?5 WHERE id = ?6",
            )?;
            for (id, pos, phasor) in batch {
                stmt.execute(params![
                    pos.w,
                    pos.x,
                    pos.y,
                    pos.z,
                    phasor.theta,
                    id.to_string()
                ])?;
            }
        }
        tx.commit()?;
        let _ = self.conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE);");
        Ok(())
    }
}
