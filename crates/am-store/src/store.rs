use std::path::Path;

use rusqlite::{Connection, params};
use uuid::Uuid;

use am_core::{DAESystem, DaemonPhasor, Episode, Neighborhood, Occurrence, Quaternion};

use crate::error::{Result, StoreError};
use crate::schema;

#[derive(Debug)]
pub struct GcResult {
    pub evicted_occurrences: u64,
    pub removed_neighborhoods: u64,
    pub removed_episodes: u64,
    pub before_occurrences: u64,
    pub before_size: u64,
    pub after_size: u64,
}

#[derive(Debug)]
pub struct ActivationStats {
    pub total: u64,
    pub zero_activation: u64,
    pub max_activation: u32,
    pub mean_activation: f64,
}

pub struct Store {
    conn: Connection,
}

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

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Verify the connection is still usable.
    pub fn health_check(&self) -> Result<()> {
        self.conn
            .execute_batch("SELECT 1")
            .map_err(StoreError::Sqlite)
    }

    /// Run a TRUNCATE checkpoint — flushes WAL and removes the file.
    /// Used during clean shutdown.
    pub fn checkpoint_truncate(&self) {
        let _ = self.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
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

    // --- Save ---

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
        // PASSIVE checkpoint after bulk write — flushes WAL without blocking readers
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

    fn save_episode_on(&self, conn: &Connection, episode: &Episode) -> Result<()> {
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
            "INSERT INTO neighborhoods (id, episode_id, seed_w, seed_x, seed_y, seed_z, source_text)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                neighborhood.id.to_string(),
                episode_id.to_string(),
                neighborhood.seed.w,
                neighborhood.seed.x,
                neighborhood.seed.y,
                neighborhood.seed.z,
                neighborhood.source_text,
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

    // --- Load ---

    pub fn load_system(&self) -> Result<DAESystem> {
        let agent_name = self
            .get_metadata("agent_name")?
            .unwrap_or_else(|| "unknown".to_string());

        let mut system = DAESystem::new(&agent_name);

        // Load all episodes
        let mut ep_stmt = self
            .conn
            .prepare("SELECT id, name, is_conscious, timestamp FROM episodes ORDER BY rowid")?;

        let episodes: Vec<(String, String, bool, String)> = ep_stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i32>(2)? != 0,
                    row.get::<_, String>(3)?,
                ))
            })?
            .collect::<std::result::Result<_, _>>()?;

        for (ep_id_str, name, is_conscious, timestamp) in episodes {
            let ep_id = parse_uuid(&ep_id_str)?;
            let neighborhoods = self.load_neighborhoods(&ep_id_str)?;

            let episode = Episode {
                id: ep_id,
                name,
                is_conscious,
                timestamp,
                neighborhoods,
            };

            if is_conscious {
                system.conscious_episode = episode;
            } else {
                system.episodes.push(episode);
            }
        }

        system.mark_dirty();
        Ok(system)
    }

    fn load_neighborhoods(&self, episode_id: &str) -> Result<Vec<Neighborhood>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, seed_w, seed_x, seed_y, seed_z, source_text
             FROM neighborhoods WHERE episode_id = ?1 ORDER BY rowid",
        )?;

        let rows: Vec<(String, f64, f64, f64, f64, String)> = stmt
            .query_map([episode_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, f64>(1)?,
                    row.get::<_, f64>(2)?,
                    row.get::<_, f64>(3)?,
                    row.get::<_, f64>(4)?,
                    row.get::<_, String>(5)?,
                ))
            })?
            .collect::<std::result::Result<_, _>>()?;

        let mut neighborhoods = Vec::with_capacity(rows.len());
        for (id_str, w, x, y, z, source_text) in rows {
            let id = parse_uuid(&id_str)?;
            let occurrences = self.load_occurrences(&id_str)?;

            neighborhoods.push(Neighborhood {
                id,
                seed: Quaternion::new(w, x, y, z),
                occurrences,
                source_text,
            });
        }

        Ok(neighborhoods)
    }

    fn load_occurrences(&self, neighborhood_id: &str) -> Result<Vec<Occurrence>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, word, pos_w, pos_x, pos_y, pos_z, phasor_theta, activation_count
             FROM occurrences WHERE neighborhood_id = ?1 ORDER BY rowid",
        )?;

        let occurrences = stmt
            .query_map([neighborhood_id], |row| {
                let id_str: String = row.get(0)?;
                let word: String = row.get(1)?;
                let w: f64 = row.get(2)?;
                let x: f64 = row.get(3)?;
                let y: f64 = row.get(4)?;
                let z: f64 = row.get(5)?;
                let theta: f64 = row.get(6)?;
                let activation_count: u32 = row.get(7)?;
                Ok((id_str, word, w, x, y, z, theta, activation_count))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        let nbhd_id = parse_uuid(neighborhood_id)?;

        occurrences
            .into_iter()
            .map(|(id_str, word, w, x, y, z, theta, activation_count)| {
                let id = parse_uuid(&id_str)?;
                Ok(Occurrence {
                    id,
                    neighborhood_id: nbhd_id,
                    word,
                    position: Quaternion::new(w, x, y, z),
                    phasor: DaemonPhasor::new(theta),
                    activation_count,
                })
            })
            .collect()
    }

    // --- Targeted updates (no full rewrite) ---

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
        let mut stmt = self
            .conn
            .prepare("SELECT user_text, assistant_text FROM conversation_buffer ORDER BY id")?;
        let rows: Vec<(String, String)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<std::result::Result<_, _>>()?;
        self.conn.execute_batch("DELETE FROM conversation_buffer")?;
        Ok(rows)
    }

    pub fn buffer_count(&self) -> Result<usize> {
        let count: usize =
            self.conn
                .query_row("SELECT COUNT(*) FROM conversation_buffer", [], |row| {
                    row.get(0)
                })?;
        Ok(count)
    }

    // --- Indexed queries ---

    pub fn get_occurrences_by_word(&self, word: &str) -> Result<Vec<Occurrence>> {
        self.load_occurrences_by_word(word)
    }

    fn load_occurrences_by_word(&self, word: &str) -> Result<Vec<Occurrence>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, neighborhood_id, word, pos_w, pos_x, pos_y, pos_z, phasor_theta, activation_count
             FROM occurrences WHERE word = ?1",
        )?;

        stmt.query_map([word], |row| {
            let id_str: String = row.get(0)?;
            let nbhd_id_str: String = row.get(1)?;
            let word: String = row.get(2)?;
            let w: f64 = row.get(3)?;
            let x: f64 = row.get(4)?;
            let y: f64 = row.get(5)?;
            let z: f64 = row.get(6)?;
            let theta: f64 = row.get(7)?;
            let activation_count: u32 = row.get(8)?;
            Ok((
                id_str,
                nbhd_id_str,
                word,
                w,
                x,
                y,
                z,
                theta,
                activation_count,
            ))
        })?
        .map(|r| {
            let (id_str, nbhd_id_str, word, w, x, y, z, theta, activation_count) = r?;
            Ok(Occurrence {
                id: parse_uuid(&id_str)?,
                neighborhood_id: parse_uuid(&nbhd_id_str)?,
                word,
                position: Quaternion::new(w, x, y, z),
                phasor: DaemonPhasor::new(theta),
                activation_count,
            })
        })
        .collect()
    }

    pub fn get_neighborhood_ids_by_word(&self, word: &str) -> Result<Vec<Uuid>> {
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT neighborhood_id FROM occurrences WHERE word = ?1")?;

        stmt.query_map([word], |row| {
            let id_str: String = row.get(0)?;
            Ok(id_str)
        })?
        .map(|r| {
            let id_str = r?;
            parse_uuid(&id_str)
        })
        .collect()
    }

    // --- Garbage collection ---

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

    /// Total occurrence count in the database.
    pub fn occurrence_count(&self) -> Result<u64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM occurrences", [], |row| row.get(0))?)
    }

    /// Run a GC pass: evict cold occurrences, clean empty structures, VACUUM.
    /// Returns (evicted_occurrences, removed_episodes).
    /// Conscious episodes (is_conscious = 1) are never touched.
    pub fn gc_pass(&self, activation_floor: u32) -> Result<GcResult> {
        let before_occs = self.occurrence_count()?;
        let before_size = self.db_size();

        let tx = self.conn.unchecked_transaction()?;

        // 1. Delete occurrences at or below the activation floor,
        //    but only from non-conscious episodes
        let evicted_occs: u64 = tx.execute(
            "DELETE FROM occurrences WHERE activation_count <= ?1
             AND neighborhood_id IN (
                 SELECT n.id FROM neighborhoods n
                 JOIN episodes e ON n.episode_id = e.id
                 WHERE e.is_conscious = 0
             )",
            [activation_floor],
        )? as u64;

        // 2. Delete neighborhoods that have no remaining occurrences
        //    (only from non-conscious episodes)
        let removed_neighborhoods: u64 = tx.execute(
            "DELETE FROM neighborhoods WHERE id NOT IN (
                 SELECT DISTINCT neighborhood_id FROM occurrences
             ) AND episode_id IN (
                 SELECT id FROM episodes WHERE is_conscious = 0
             )",
            [],
        )? as u64;

        // 3. Delete episodes that have no remaining neighborhoods
        //    (only non-conscious)
        let removed_episodes: u64 = tx.execute(
            "DELETE FROM episodes WHERE is_conscious = 0
             AND id NOT IN (
                 SELECT DISTINCT episode_id FROM neighborhoods
             )",
            [],
        )? as u64;

        tx.commit()?;

        // 4. VACUUM to reclaim disk space (must run outside transaction)
        let _ = self.conn.execute_batch("VACUUM;");

        let after_size = self.db_size();

        Ok(GcResult {
            evicted_occurrences: evicted_occs,
            removed_neighborhoods,
            removed_episodes,
            before_occurrences: before_occs,
            before_size,
            after_size,
        })
    }

    /// Aggressive GC: evict coldest occurrences until DB is under target size.
    /// Only used when activation-floor eviction wasn't sufficient.
    /// Conscious episodes are never touched.
    pub fn gc_to_target_size(&self, target_bytes: u64) -> Result<GcResult> {
        let before_occs = self.occurrence_count()?;
        let before_size = self.db_size();

        // Get activation counts in ascending order (coldest first), non-conscious only
        let mut stmt = self.conn.prepare(
            "SELECT o.id, o.activation_count FROM occurrences o
             JOIN neighborhoods n ON o.neighborhood_id = n.id
             JOIN episodes e ON n.episode_id = e.id
             WHERE e.is_conscious = 0
             ORDER BY o.activation_count ASC",
        )?;

        let rows: Vec<(String, u32)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<std::result::Result<_, _>>()?;

        if rows.is_empty() {
            return Ok(GcResult {
                evicted_occurrences: 0,
                removed_neighborhoods: 0,
                removed_episodes: 0,
                before_occurrences: before_occs,
                before_size,
                after_size: before_size,
            });
        }

        // Estimate bytes per occurrence: total_size / total_occurrences
        let total_occs = before_occs.max(1);
        let bytes_per_occ = before_size / total_occs;

        // Calculate how many we need to evict
        let excess = before_size.saturating_sub(target_bytes);
        let to_evict = if bytes_per_occ > 0 {
            (excess / bytes_per_occ).min(rows.len() as u64)
        } else {
            0
        };

        if to_evict == 0 {
            return Ok(GcResult {
                evicted_occurrences: 0,
                removed_neighborhoods: 0,
                removed_episodes: 0,
                before_occurrences: before_occs,
                before_size,
                after_size: before_size,
            });
        }

        // Delete the coldest occurrences + clean up empty structures atomically
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut del_stmt = tx.prepare("DELETE FROM occurrences WHERE id = ?1")?;
            for (id, _) in rows.iter().take(to_evict as usize) {
                del_stmt.execute([id])?;
            }
        }

        let removed_neighborhoods: u64 = tx.execute(
            "DELETE FROM neighborhoods WHERE id NOT IN (
                 SELECT DISTINCT neighborhood_id FROM occurrences
             ) AND episode_id IN (
                 SELECT id FROM episodes WHERE is_conscious = 0
             )",
            [],
        )? as u64;

        let removed_episodes: u64 = tx.execute(
            "DELETE FROM episodes WHERE is_conscious = 0
             AND id NOT IN (
                 SELECT DISTINCT episode_id FROM neighborhoods
             )",
            [],
        )? as u64;

        tx.commit()?;

        // VACUUM to reclaim disk space (must run outside transaction)
        let _ = self.conn.execute_batch("VACUUM;");
        let after_size = self.db_size();

        Ok(GcResult {
            evicted_occurrences: to_evict,
            removed_neighborhoods,
            removed_episodes,
            before_occurrences: before_occs,
            before_size,
            after_size,
        })
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
}

impl Drop for Store {
    fn drop(&mut self) {
        // Clean shutdown: flush WAL to main DB
        let _ = self.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
    }
}

fn parse_uuid(s: &str) -> Result<Uuid> {
    Uuid::parse_str(s).map_err(|e| StoreError::InvalidData(format!("invalid UUID '{s}': {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use am_core::Neighborhood;
    use rand::SeedableRng;
    use rand::rngs::SmallRng;

    fn rng() -> SmallRng {
        SmallRng::seed_from_u64(42)
    }

    fn to_tokens(words: &[&str]) -> Vec<String> {
        words.iter().map(|s| s.to_string()).collect()
    }

    fn make_system() -> DAESystem {
        let mut rng = rng();
        let mut sys = DAESystem::new("test-agent");

        let mut ep1 = Episode::new("episode-1");
        let tokens = to_tokens(&["hello", "world", "test"]);
        let n = Neighborhood::from_tokens(&tokens, None, "hello world test", &mut rng);
        ep1.add_neighborhood(n);
        sys.add_episode(ep1);

        sys.add_to_conscious("conscious thought", &mut rng);

        sys
    }

    #[test]
    fn test_save_and_load_roundtrip() {
        let store = Store::open_in_memory().unwrap();
        let original = make_system();

        store.save_system(&original).unwrap();
        let loaded = store.load_system().unwrap();

        assert_eq!(loaded.agent_name, "test-agent");
        assert_eq!(loaded.episodes.len(), 1);
        assert_eq!(loaded.episodes[0].name, "episode-1");
        assert_eq!(loaded.episodes[0].neighborhoods.len(), 1);
        assert_eq!(loaded.episodes[0].neighborhoods[0].occurrences.len(), 3);
        assert!(loaded.conscious_episode.is_conscious);
        assert_eq!(loaded.conscious_episode.neighborhoods.len(), 1);
    }

    #[test]
    fn test_quaternion_precision_roundtrip() {
        let store = Store::open_in_memory().unwrap();
        let original = make_system();

        let orig_pos = original.episodes[0].neighborhoods[0].occurrences[0].position;

        store.save_system(&original).unwrap();
        let loaded = store.load_system().unwrap();

        let loaded_pos = loaded.episodes[0].neighborhoods[0].occurrences[0].position;
        let dist = orig_pos.angular_distance(loaded_pos);
        assert!(dist < 1e-10, "quaternion drift: {dist}");
    }

    #[test]
    fn test_phasor_roundtrip() {
        let store = Store::open_in_memory().unwrap();
        let original = make_system();

        let orig_theta = original.episodes[0].neighborhoods[0].occurrences[0]
            .phasor
            .theta;

        store.save_system(&original).unwrap();
        let loaded = store.load_system().unwrap();

        let loaded_theta = loaded.episodes[0].neighborhoods[0].occurrences[0]
            .phasor
            .theta;
        assert!(
            (orig_theta - loaded_theta).abs() < 1e-10,
            "phasor drift: {} vs {}",
            orig_theta,
            loaded_theta
        );
    }

    #[test]
    fn test_increment_activation() {
        let store = Store::open_in_memory().unwrap();
        let system = make_system();
        store.save_system(&system).unwrap();

        let occ_id = system.episodes[0].neighborhoods[0].occurrences[0].id;

        store.increment_activation(occ_id).unwrap();
        store.increment_activation(occ_id).unwrap();

        let loaded = store.load_system().unwrap();
        let loaded_count = loaded.episodes[0].neighborhoods[0].occurrences[0].activation_count;
        assert_eq!(loaded_count, 2);
    }

    #[test]
    fn test_increment_activation_nonexistent() {
        let store = Store::open_in_memory().unwrap();
        let result = store.increment_activation(Uuid::new_v4());
        assert!(result.is_err());
    }

    #[test]
    fn test_get_occurrences_by_word() {
        let store = Store::open_in_memory().unwrap();
        let system = make_system();
        store.save_system(&system).unwrap();

        let occs = store.get_occurrences_by_word("hello").unwrap();
        assert_eq!(occs.len(), 1);
        assert_eq!(occs[0].word, "hello");

        let none = store.get_occurrences_by_word("nonexistent").unwrap();
        assert!(none.is_empty());
    }

    #[test]
    fn test_get_neighborhood_ids_by_word() {
        let store = Store::open_in_memory().unwrap();
        let system = make_system();
        store.save_system(&system).unwrap();

        let ids = store.get_neighborhood_ids_by_word("hello").unwrap();
        assert_eq!(ids.len(), 1);
    }

    #[test]
    fn test_save_occurrence_positions() {
        let store = Store::open_in_memory().unwrap();
        let system = make_system();
        store.save_system(&system).unwrap();

        let occ = &system.episodes[0].neighborhoods[0].occurrences[0];
        let new_pos = Quaternion::new(0.5, 0.5, 0.5, 0.5);
        let new_phasor = DaemonPhasor::new(1.23);

        store
            .save_occurrence_positions(&[(occ.id, new_pos, new_phasor)])
            .unwrap();

        let loaded = store.load_system().unwrap();
        let loaded_occ = &loaded.episodes[0].neighborhoods[0].occurrences[0];
        let dist = new_pos.angular_distance(loaded_occ.position);
        assert!(dist < 1e-10, "position not updated: {dist}");
        assert!(
            (loaded_occ.phasor.theta - 1.23).abs() < 1e-10,
            "phasor not updated"
        );
    }

    #[test]
    fn test_metadata() {
        let store = Store::open_in_memory().unwrap();

        assert!(store.get_metadata("foo").unwrap().is_none());

        store.set_metadata("foo", "bar").unwrap();
        assert_eq!(store.get_metadata("foo").unwrap(), Some("bar".to_string()));

        store.set_metadata("foo", "baz").unwrap();
        assert_eq!(store.get_metadata("foo").unwrap(), Some("baz".to_string()));
    }

    #[test]
    fn test_save_overwrites_previous() {
        let store = Store::open_in_memory().unwrap();
        let system = make_system();

        store.save_system(&system).unwrap();
        store.save_system(&system).unwrap();

        let loaded = store.load_system().unwrap();
        assert_eq!(loaded.episodes.len(), 1);
    }

    #[test]
    fn test_load_empty_db() {
        let store = Store::open_in_memory().unwrap();
        let system = store.load_system().unwrap();
        assert_eq!(system.agent_name, "unknown");
        assert!(system.episodes.is_empty());
        assert!(system.conscious_episode.is_conscious);
    }

    #[test]
    fn test_activation_count_preserved() {
        let store = Store::open_in_memory().unwrap();
        let mut system = make_system();

        // Pre-activate conscious occurrences are already at 1
        // Subconscious at 0
        system.episodes[0].neighborhoods[0].occurrences[0].activation_count = 42;

        store.save_system(&system).unwrap();
        let loaded = store.load_system().unwrap();

        assert_eq!(
            loaded.episodes[0].neighborhoods[0].occurrences[0].activation_count,
            42
        );
    }

    #[test]
    fn test_health_check() {
        let store = Store::open_in_memory().unwrap();
        assert!(store.health_check().is_ok());
    }

    // --- GC tests ---

    fn make_system_with_activations() -> DAESystem {
        let mut rng = rng();
        let mut sys = DAESystem::new("test-agent");

        let mut ep1 = Episode::new("episode-cold");
        let tokens = to_tokens(&["cold", "unused", "stale"]);
        let n = Neighborhood::from_tokens(&tokens, None, "cold unused stale", &mut rng);
        ep1.add_neighborhood(n);
        sys.add_episode(ep1);

        let mut ep2 = Episode::new("episode-warm");
        let tokens = to_tokens(&["warm", "active"]);
        let mut n = Neighborhood::from_tokens(&tokens, None, "warm active", &mut rng);
        // Activate these occurrences
        for occ in &mut n.occurrences {
            occ.activation_count = 5;
        }
        ep2.add_neighborhood(n);
        sys.add_episode(ep2);

        // Add conscious memory (should never be GC'd)
        sys.add_to_conscious("protected insight", &mut rng);

        sys
    }

    #[test]
    fn test_gc_evicts_cold_occurrences() {
        let store = Store::open_in_memory().unwrap();
        let sys = make_system_with_activations();
        store.save_system(&sys).unwrap();

        // Before GC: 3 cold (activation=0) + 2 warm (activation=5) + conscious
        let before = store.occurrence_count().unwrap();
        assert!(before >= 5);

        // Evict occurrences with activation_count <= 0
        let result = store.gc_pass(0).unwrap();
        assert_eq!(
            result.evicted_occurrences, 3,
            "should evict 3 cold occurrences"
        );

        // After GC: warm + conscious should remain
        let loaded = store.load_system().unwrap();
        assert_eq!(loaded.episodes.len(), 1, "cold episode should be removed");
        assert_eq!(loaded.episodes[0].name, "episode-warm");
        assert!(
            !loaded.conscious_episode.neighborhoods.is_empty(),
            "conscious should survive GC"
        );
    }

    #[test]
    fn test_gc_preserves_conscious() {
        let store = Store::open_in_memory().unwrap();
        let mut rng = rng();
        let mut sys = DAESystem::new("test-agent");

        // Only conscious memory, no subconscious episodes
        sys.add_to_conscious("precious insight", &mut rng);
        store.save_system(&sys).unwrap();

        let result = store.gc_pass(0).unwrap();
        assert_eq!(
            result.evicted_occurrences, 0,
            "conscious should never be evicted"
        );

        let loaded = store.load_system().unwrap();
        assert!(
            !loaded.conscious_episode.neighborhoods.is_empty(),
            "conscious should survive"
        );
    }

    #[test]
    fn test_gc_removes_empty_episodes() {
        let store = Store::open_in_memory().unwrap();
        let sys = make_system_with_activations();
        store.save_system(&sys).unwrap();

        let result = store.gc_pass(0).unwrap();
        assert_eq!(result.removed_episodes, 1, "episode-cold should be removed");
        assert_eq!(result.removed_neighborhoods, 1);
    }

    #[test]
    fn test_activation_distribution() {
        let store = Store::open_in_memory().unwrap();
        let sys = make_system_with_activations();
        store.save_system(&sys).unwrap();

        let stats = store.activation_distribution().unwrap();
        assert!(stats.total >= 5);
        assert!(stats.zero_activation >= 3); // cold occurrences
        assert_eq!(stats.max_activation, 5); // warm occurrences
        assert!(stats.mean_activation > 0.0);
    }

    #[test]
    fn test_gc_noop_when_empty() {
        let store = Store::open_in_memory().unwrap();
        // No data saved — empty DB
        let result = store.gc_pass(0).unwrap();
        assert_eq!(result.evicted_occurrences, 0);
        assert_eq!(result.removed_episodes, 0);
    }
}
