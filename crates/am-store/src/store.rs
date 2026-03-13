use std::path::Path;

use rusqlite::{Connection, params};
use uuid::Uuid;

use am_core::{
    DAESystem, DaemonPhasor, Episode, Neighborhood, NeighborhoodType, Occurrence, Quaternion,
};

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

    // --- Load ---

    pub fn load_system(&self) -> Result<DAESystem> {
        let agent_name = self
            .get_metadata("agent_name")?
            .unwrap_or_else(|| "unknown".to_string());

        let mut system = DAESystem::new(&agent_name);

        // Single three-way JOIN replaces the previous 1 + N + N*M query pattern.
        // LEFT JOINs handle episodes with no neighborhoods and neighborhoods with no occurrences.
        let mut stmt = self.conn.prepare(
            "SELECT e.id, e.name, e.is_conscious, e.timestamp,
                    n.id, n.seed_w, n.seed_x, n.seed_y, n.seed_z,
                    n.source_text, COALESCE(n.neighborhood_type, 'memory'),
                    n.epoch, n.superseded_by,
                    o.id, o.word, o.pos_w, o.pos_x, o.pos_y, o.pos_z,
                    o.phasor_theta, o.activation_count
             FROM episodes e
             LEFT JOIN neighborhoods n ON n.episode_id = e.id
             LEFT JOIN occurrences o ON o.neighborhood_id = n.id
             ORDER BY e.rowid, n.rowid, o.rowid",
        )?;

        // Track current episode and neighborhood being assembled.
        let mut current_ep_id: Option<String> = None;
        let mut current_nbhd_id: Option<String> = None;
        let mut current_episode: Option<Episode> = None;
        let mut current_nbhd: Option<Neighborhood> = None;

        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let ep_id_str: String = row.get(0)?;
            let nbhd_id_str: Option<String> = row.get(4)?;
            let occ_id_str: Option<String> = row.get(13)?;

            // Episode boundary: flush previous neighborhood and episode
            if current_ep_id.as_ref() != Some(&ep_id_str) {
                if let Some(nbhd) = current_nbhd.take()
                    && let Some(ep) = current_episode.as_mut()
                {
                    ep.neighborhoods.push(nbhd);
                }
                current_nbhd_id = None;
                if let Some(ep) = current_episode.take() {
                    if ep.is_conscious {
                        system.conscious_episode = ep;
                    } else {
                        system.episodes.push(ep);
                    }
                }

                let ep_id = parse_uuid(&ep_id_str)?;
                current_episode = Some(Episode {
                    id: ep_id,
                    name: row.get(1)?,
                    is_conscious: row.get::<_, i32>(2)? != 0,
                    timestamp: row.get(3)?,
                    neighborhoods: Vec::new(),
                });
                current_ep_id = Some(ep_id_str);
            }

            // Neighborhood boundary
            if let Some(ref nid) = nbhd_id_str
                && current_nbhd_id.as_ref() != Some(nid)
            {
                if let Some(nbhd) = current_nbhd.take()
                    && let Some(ep) = current_episode.as_mut()
                {
                    ep.neighborhoods.push(nbhd);
                }

                let id = parse_uuid(nid)?;
                let superseded_by: Option<String> = row.get(12)?;
                current_nbhd = Some(Neighborhood {
                    id,
                    seed: Quaternion::new(row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?),
                    occurrences: Vec::new(),
                    source_text: row.get(9)?,
                    neighborhood_type: NeighborhoodType::from_str_lossy(&row.get::<_, String>(10)?),
                    epoch: row.get(11)?,
                    superseded_by: superseded_by.and_then(|s| Uuid::parse_str(&s).ok()),
                });
                current_nbhd_id = Some(nid.clone());
            }

            // Occurrence row
            if let (Some(oid), Some(nid)) = (&occ_id_str, &nbhd_id_str) {
                let id = parse_uuid(oid)?;
                let nbhd_uuid = parse_uuid(nid)?;
                if let Some(nbhd) = current_nbhd.as_mut() {
                    nbhd.occurrences.push(Occurrence {
                        id,
                        neighborhood_id: nbhd_uuid,
                        word: row.get(14)?,
                        position: Quaternion::new(
                            row.get(15)?,
                            row.get(16)?,
                            row.get(17)?,
                            row.get(18)?,
                        ),
                        phasor: DaemonPhasor::new(row.get(19)?),
                        activation_count: row.get(20)?,
                    });
                }
            }
        }

        // Flush remaining neighborhood and episode
        if let Some(nbhd) = current_nbhd.take()
            && let Some(ep) = current_episode.as_mut()
        {
            ep.neighborhoods.push(nbhd);
        }
        if let Some(ep) = current_episode.take() {
            if ep.is_conscious {
                system.conscious_episode = ep;
            } else {
                system.episodes.push(ep);
            }
        }

        system.mark_dirty();
        system.sync_next_epoch();
        Ok(system)
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
        let tx = self.conn.unchecked_transaction()?;

        let mut stmt = tx
            .prepare("SELECT id, user_text, assistant_text FROM conversation_buffer ORDER BY id")?;
        let entries: Vec<(i64, String, String)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect::<std::result::Result<_, _>>()?;
        drop(stmt);

        if !entries.is_empty() {
            // Safe to delete all rows: the transaction guarantees no new entries
            // appear between the SELECT and DELETE, and we read every row above.
            tx.execute("DELETE FROM conversation_buffer", [])?;
        }

        tx.commit()?;

        Ok(entries.into_iter().map(|(_, u, a)| (u, a)).collect())
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

    // --- Inspection queries (SQL-level, no full system load) ---

    /// List all episodes with summary stats.
    pub fn list_episodes(&self) -> Result<Vec<EpisodeInfo>> {
        let mut stmt = self.conn.prepare(
            "SELECT e.id, e.name, e.is_conscious, e.timestamp,
                    COUNT(DISTINCT n.id) as nbhd_count,
                    COUNT(o.id) as occ_count,
                    COALESCE(SUM(o.activation_count), 0) as total_activation
             FROM episodes e
             LEFT JOIN neighborhoods n ON n.episode_id = e.id
             LEFT JOIN occurrences o ON o.neighborhood_id = n.id
             GROUP BY e.id
             ORDER BY e.is_conscious DESC, e.rowid",
        )?;

        let rows = stmt
            .query_map([], |row| {
                Ok(EpisodeInfo {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    is_conscious: row.get::<_, i32>(2)? != 0,
                    timestamp: row.get(3)?,
                    neighborhood_count: row.get(4)?,
                    occurrence_count: row.get(5)?,
                    total_activation: row.get(6)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(rows)
    }

    /// List conscious neighborhoods with their source text.
    pub fn list_conscious_neighborhoods(&self) -> Result<Vec<NeighborhoodInfo>> {
        let mut stmt = self.conn.prepare(
            "SELECT n.id, n.source_text, COUNT(o.id) as occ_count,
                    COALESCE(SUM(o.activation_count), 0) as total_activation
             FROM neighborhoods n
             JOIN episodes e ON n.episode_id = e.id
             LEFT JOIN occurrences o ON o.neighborhood_id = n.id
             WHERE e.is_conscious = 1
             GROUP BY n.id
             ORDER BY n.rowid",
        )?;

        let rows = stmt
            .query_map([], |row| {
                Ok(NeighborhoodInfo {
                    id: row.get(0)?,
                    source_text: row.get(1)?,
                    occurrence_count: row.get(2)?,
                    total_activation: row.get(3)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(rows)
    }

    /// List all neighborhoods (across all episodes).
    pub fn list_neighborhoods(&self) -> Result<Vec<NeighborhoodDetail>> {
        let mut stmt = self.conn.prepare(
            "SELECT n.id, n.source_text, e.name, e.is_conscious,
                    COUNT(o.id) as occ_count,
                    COALESCE(SUM(o.activation_count), 0) as total_activation,
                    COALESCE(MAX(o.activation_count), 0) as max_activation
             FROM neighborhoods n
             JOIN episodes e ON n.episode_id = e.id
             LEFT JOIN occurrences o ON o.neighborhood_id = n.id
             GROUP BY n.id
             ORDER BY total_activation DESC",
        )?;

        let rows = stmt
            .query_map([], |row| {
                Ok(NeighborhoodDetail {
                    id: row.get(0)?,
                    source_text: row.get(1)?,
                    episode_name: row.get(2)?,
                    is_conscious: row.get::<_, i32>(3)? != 0,
                    occurrence_count: row.get(4)?,
                    total_activation: row.get(5)?,
                    max_activation: row.get(6)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(rows)
    }

    /// Get top words by activation count.
    pub fn top_words(&self, limit: usize) -> Result<Vec<(String, u32, u64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT word, SUM(activation_count) as total_act, COUNT(*) as occ_count
             FROM occurrences
             GROUP BY word
             ORDER BY total_act DESC
             LIMIT ?1",
        )?;

        let rows = stmt
            .query_map([limit as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, u32>(1)?,
                    row.get::<_, u64>(2)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(rows)
    }

    /// Count unique words in the database.
    pub fn unique_word_count(&self) -> Result<u64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(DISTINCT word) FROM occurrences", [], |row| {
                row.get(0)
            })?)
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

    /// Total neighborhood count in the database.
    pub fn neighborhood_count(&self) -> Result<u64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM neighborhoods", [], |row| row.get(0))?)
    }

    /// Count occurrences eligible for GC eviction at the given activation floor.
    /// Excludes conscious episodes.
    pub fn gc_eligible_count(&self, activation_floor: u32) -> Result<u64> {
        let count: u64 = self.conn.query_row(
            "SELECT COUNT(*) FROM occurrences o
             JOIN neighborhoods n ON o.neighborhood_id = n.id
             JOIN episodes e ON n.episode_id = e.id
             WHERE e.is_conscious = 0 AND o.activation_count <= ?1",
            [activation_floor],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Run a GC pass: evict cold occurrences, clean empty structures, VACUUM.
    /// Returns (evicted_occurrences, removed_episodes).
    /// Conscious episodes (is_conscious = 1) are never touched.
    /// Respects retention policy: grace epoch window and retention days.
    pub fn gc_pass(
        &self,
        activation_floor: u32,
        retention: &crate::config::RetentionPolicy,
    ) -> Result<GcResult> {
        // Early return if below min_neighborhoods floor
        let total_nbhds = self.neighborhood_count()?;
        if total_nbhds < retention.min_neighborhoods {
            return Ok(GcResult {
                evicted_occurrences: 0,
                removed_neighborhoods: 0,
                removed_episodes: 0,
                before_occurrences: self.occurrence_count()?,
                before_size: self.db_size(),
                after_size: self.db_size(),
            });
        }

        let before_occs = self.occurrence_count()?;
        let before_size = self.db_size();

        // Compute retention parameters. When a retention dimension is disabled,
        // use sentinel -1 which makes the SQL clause a no-op via short-circuit:
        //   ?2 = -1 bypasses the epoch filter
        //   ?3 = -1 bypasses the timestamp filter
        let epoch_floor: i64 = if retention.grace_epochs > 0 {
            let max_epoch: u64 = self
                .conn
                .query_row(
                    "SELECT COALESCE(MAX(epoch), 0) FROM neighborhoods",
                    [],
                    |row| row.get(0),
                )
                .unwrap_or(0);
            max_epoch.saturating_sub(retention.grace_epochs) as i64
        } else {
            -1
        };
        let retention_secs: i64 = if retention.retention_days > 0 {
            (retention.retention_days as i64) * 86400
        } else {
            -1
        };

        let tx = self.conn.unchecked_transaction()?;

        // 1. Delete occurrences at or below the activation floor,
        //    but only from non-conscious episodes, and respecting retention.
        // Fixed SQL shape: ?2 = -1 disables epoch check, ?3 = -1 disables retention check.
        let evicted_occs: u64 = tx.execute(
            "DELETE FROM occurrences WHERE activation_count <= ?1
             AND neighborhood_id IN (
                 SELECT n.id FROM neighborhoods n
                 JOIN episodes e ON n.episode_id = e.id
                 WHERE e.is_conscious = 0
                   AND (?2 = -1 OR n.epoch < ?2)
                   AND (?3 = -1 OR e.timestamp = ''
                        OR REPLACE(REPLACE(e.timestamp, 'T', ' '), 'Z', '')
                           < datetime('now', '-' || ?3 || ' seconds'))
             )",
            rusqlite::params![activation_floor, epoch_floor, retention_secs],
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
    /// Uses composite eviction score: lower activation and older epoch = evicted first.
    pub fn gc_to_target_size(
        &self,
        target_bytes: u64,
        retention: &crate::config::RetentionPolicy,
    ) -> Result<GcResult> {
        let before_occs = self.occurrence_count()?;
        let before_size = self.db_size();

        // Build retention filter clauses with parameterized values.
        // Parameters: ?1 = max_epoch_f, ?2 = recency_weight, then dynamic retention params.
        let max_epoch: u64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(epoch), 0) FROM neighborhoods",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        let max_epoch_f = (max_epoch as f64).max(1.0);

        // Sentinel -1 disables the clause via short-circuit in SQL.
        let epoch_floor: i64 = if retention.grace_epochs > 0 {
            max_epoch.saturating_sub(retention.grace_epochs) as i64
        } else {
            -1
        };
        let retention_secs: i64 = if retention.retention_days > 0 {
            (retention.retention_days as i64) * 86400
        } else {
            -1
        };

        // Get occurrences sorted by composite eviction score (most evictable first).
        // Score = activation_count - (epoch / max_epoch) * recency_weight
        // Lower score = higher eviction priority.
        // Fixed SQL shape: ?3 = -1 disables epoch check, ?4 = -1 disables retention check.
        let mut stmt = self.conn.prepare(
            "SELECT o.id, o.activation_count FROM occurrences o
                 JOIN neighborhoods n ON o.neighborhood_id = n.id
                 JOIN episodes e ON n.episode_id = e.id
                 WHERE e.is_conscious = 0
                   AND (?3 = -1 OR n.epoch < ?3)
                   AND (?4 = -1 OR e.timestamp = ''
                        OR REPLACE(REPLACE(e.timestamp, 'T', ' '), 'Z', '')
                           < datetime('now', '-' || ?4 || ' seconds'))
                 ORDER BY (o.activation_count - (CAST(n.epoch AS REAL) / ?1) * ?2) ASC",
        )?;

        let rows: Vec<(String, u32)> = stmt
            .query_map(
                rusqlite::params![
                    max_epoch_f,
                    retention.recency_weight,
                    epoch_floor,
                    retention_secs
                ],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?
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

    // --- Forget (targeted removal) ---

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

    /// Permissive retention policy that disables all protection for testing.
    fn no_retention() -> crate::config::RetentionPolicy {
        crate::config::RetentionPolicy {
            grace_epochs: 0,
            retention_days: 0,
            min_neighborhoods: 0,
            recency_weight: 0.0,
        }
    }

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
        let result = store.gc_pass(0, &no_retention()).unwrap();
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

        let result = store.gc_pass(0, &no_retention()).unwrap();
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

        let result = store.gc_pass(0, &no_retention()).unwrap();
        assert_eq!(result.removed_episodes, 1, "episode-cold should be removed");
        assert_eq!(result.removed_neighborhoods, 1);
    }

    #[test]
    fn test_gc_grace_epochs_protects_fresh_data() {
        let store = Store::open_in_memory().unwrap();
        let sys = make_system_with_activations();
        store.save_system(&sys).unwrap();

        // Grace window of 100 epochs covers everything - nothing should be evicted
        let policy = crate::config::RetentionPolicy {
            grace_epochs: 100,
            retention_days: 0,
            min_neighborhoods: 0,
            recency_weight: 0.0,
        };
        let result = store.gc_pass(0, &policy).unwrap();
        assert_eq!(
            result.evicted_occurrences, 0,
            "grace window should protect all neighborhoods"
        );
    }

    #[test]
    fn test_gc_min_neighborhoods_prevents_gc() {
        let store = Store::open_in_memory().unwrap();
        let sys = make_system_with_activations();
        store.save_system(&sys).unwrap();

        // min_neighborhoods higher than what exists - GC should be skipped
        let policy = crate::config::RetentionPolicy {
            grace_epochs: 0,
            retention_days: 0,
            min_neighborhoods: 1000,
            recency_weight: 0.0,
        };
        let result = store.gc_pass(0, &policy).unwrap();
        assert_eq!(
            result.evicted_occurrences, 0,
            "min_neighborhoods floor should prevent GC"
        );
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
        // No data saved - empty DB
        let result = store.gc_pass(0, &no_retention()).unwrap();
        assert_eq!(result.evicted_occurrences, 0);
        assert_eq!(result.removed_episodes, 0);
    }

    // --- Inspection query tests ---

    #[test]
    fn test_list_episodes() {
        let store = Store::open_in_memory().unwrap();
        let sys = make_system();
        store.save_system(&sys).unwrap();

        let episodes = store.list_episodes().unwrap();
        // 1 subconscious + 1 conscious
        assert_eq!(episodes.len(), 2);

        let conscious: Vec<_> = episodes.iter().filter(|e| e.is_conscious).collect();
        assert_eq!(conscious.len(), 1);
        assert!(conscious[0].occurrence_count > 0);

        let sub: Vec<_> = episodes.iter().filter(|e| !e.is_conscious).collect();
        assert_eq!(sub.len(), 1);
        assert_eq!(sub[0].name, "episode-1");
        assert_eq!(sub[0].neighborhood_count, 1);
        assert_eq!(sub[0].occurrence_count, 3);
    }

    #[test]
    fn test_list_conscious_neighborhoods() {
        let store = Store::open_in_memory().unwrap();
        let sys = make_system();
        store.save_system(&sys).unwrap();

        let conscious = store.list_conscious_neighborhoods().unwrap();
        assert_eq!(conscious.len(), 1);
        assert_eq!(conscious[0].source_text, "conscious thought");
        assert!(conscious[0].occurrence_count > 0);
    }

    #[test]
    fn test_list_neighborhoods() {
        let store = Store::open_in_memory().unwrap();
        let sys = make_system();
        store.save_system(&sys).unwrap();

        let all = store.list_neighborhoods().unwrap();
        // 1 subconscious + 1 conscious neighborhood
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn test_top_words() {
        let store = Store::open_in_memory().unwrap();
        let sys = make_system_with_activations();
        store.save_system(&sys).unwrap();

        let top = store.top_words(3).unwrap();
        assert!(!top.is_empty());
        // "warm" and "active" have activation=5 each, should be at top
        let first_activation = top[0].1;
        assert!(first_activation >= 5);
    }

    #[test]
    fn test_unique_word_count() {
        let store = Store::open_in_memory().unwrap();
        let sys = make_system();
        store.save_system(&sys).unwrap();

        let count = store.unique_word_count().unwrap();
        // "hello", "world", "test" + conscious words
        assert!(count >= 3);
    }

    #[test]
    fn test_list_episodes_empty() {
        let store = Store::open_in_memory().unwrap();
        let episodes = store.list_episodes().unwrap();
        assert!(episodes.is_empty());
    }

    #[test]
    fn test_list_conscious_empty() {
        let store = Store::open_in_memory().unwrap();
        let conscious = store.list_conscious_neighborhoods().unwrap();
        assert!(conscious.is_empty());
    }

    #[test]
    fn test_forget_episode() {
        let store = Store::open_in_memory().unwrap();
        let sys = make_system();
        store.save_system(&sys).unwrap();

        let episodes = store.list_episodes().unwrap();
        // 2 episodes: 1 subconscious + 1 conscious
        assert_eq!(episodes.len(), 2);

        let sub_ep = episodes.iter().find(|e| !e.is_conscious).unwrap();
        let before = store.occurrence_count().unwrap();
        let removed = store.forget_episode(&sub_ep.id).unwrap();
        assert!(removed > 0);
        assert_eq!(store.occurrence_count().unwrap(), before - removed);

        // Only conscious episode should remain
        let after = store.list_episodes().unwrap();
        assert_eq!(after.len(), 1);
        assert!(after[0].is_conscious);
    }

    #[test]
    fn test_forget_episode_not_found() {
        let store = Store::open_in_memory().unwrap();
        let sys = make_system();
        store.save_system(&sys).unwrap();

        let removed = store
            .forget_episode("00000000-0000-0000-0000-000000000000")
            .unwrap();
        assert_eq!(removed, 0);
    }

    #[test]
    fn test_forget_conscious() {
        let store = Store::open_in_memory().unwrap();
        let sys = make_system();
        store.save_system(&sys).unwrap();

        let conscious = store.list_conscious_neighborhoods().unwrap();
        assert!(!conscious.is_empty());

        let removed = store.forget_conscious(&conscious[0].id).unwrap();
        assert!(removed > 0);

        let after = store.list_conscious_neighborhoods().unwrap();
        assert!(after.is_empty());
    }

    #[test]
    fn test_forget_conscious_rejects_subconscious() {
        let store = Store::open_in_memory().unwrap();
        let sys = make_system();
        store.save_system(&sys).unwrap();

        let _episodes = store.list_episodes().unwrap();
        // Get a neighborhood from a subconscious episode
        let neighborhoods = store.list_neighborhoods().unwrap();
        let sub_nbhd = neighborhoods
            .iter()
            .find(|n| n.episode_name != "conscious")
            .unwrap();

        let result = store.forget_conscious(&sub_nbhd.id);
        assert!(result.is_err());
    }

    #[test]
    fn test_forget_term() {
        let store = Store::open_in_memory().unwrap();
        let sys = make_system();
        store.save_system(&sys).unwrap();

        let before = store.occurrence_count().unwrap();
        let (removed_occs, _, _) = store.forget_term("hello").unwrap();
        assert!(removed_occs > 0);
        assert!(store.occurrence_count().unwrap() < before);
    }

    #[test]
    fn test_forget_term_not_found() {
        let store = Store::open_in_memory().unwrap();
        let sys = make_system();
        store.save_system(&sys).unwrap();

        let (removed, _, _) = store.forget_term("nonexistent").unwrap();
        assert_eq!(removed, 0);
    }

    #[test]
    fn test_drain_buffer_idempotent() {
        let store = Store::open_in_memory().unwrap();
        store.append_buffer("hello", "world").unwrap();
        store.append_buffer("foo", "bar").unwrap();

        let first = store.drain_buffer().unwrap();
        assert_eq!(first.len(), 2);
        assert_eq!(first[0], ("hello".to_string(), "world".to_string()));
        assert_eq!(first[1], ("foo".to_string(), "bar".to_string()));

        // Second drain returns empty: rows were deleted atomically
        let second = store.drain_buffer().unwrap();
        assert!(second.is_empty(), "second drain should return empty");
    }

    /// Regression test for ALP-1239: drain_buffer atomicity.
    ///
    /// The pre-fix implementation performed SELECT then DELETE without a
    /// transaction, creating a crash window where rows could be deleted from
    /// the database but never returned to the caller. The fix wraps both
    /// operations in a single transaction so they commit atomically.
    ///
    /// This test verifies the data integrity invariant: every buffered row
    /// is returned exactly once across interleaved append/drain cycles, with
    /// buffer_count staying consistent at each step. The pre-fix code could
    /// violate this invariant under concurrent access or crash recovery.
    #[test]
    fn test_drain_buffer_atomicity_no_lost_rows() {
        let store = Store::open_in_memory().unwrap();

        // Phase 1: buffer 5 entries, drain, verify all returned and count is 0
        for i in 0..5 {
            store
                .append_buffer(&format!("user_{i}"), &format!("asst_{i}"))
                .unwrap();
        }
        assert_eq!(store.buffer_count().unwrap(), 5);

        let drained = store.drain_buffer().unwrap();
        assert_eq!(drained.len(), 5, "all 5 rows must be returned");
        assert_eq!(
            store.buffer_count().unwrap(),
            0,
            "buffer must be empty after drain"
        );

        // Verify exact content and ordering
        for (i, (user, asst)) in drained.iter().enumerate() {
            assert_eq!(user, &format!("user_{i}"));
            assert_eq!(asst, &format!("asst_{i}"));
        }

        // Phase 2: interleave appends and drains
        store.append_buffer("a", "1").unwrap();
        store.append_buffer("b", "2").unwrap();
        assert_eq!(store.buffer_count().unwrap(), 2);

        let batch1 = store.drain_buffer().unwrap();
        assert_eq!(batch1.len(), 2);
        assert_eq!(store.buffer_count().unwrap(), 0);

        // Drain on empty is safe
        let empty = store.drain_buffer().unwrap();
        assert!(empty.is_empty());
        assert_eq!(store.buffer_count().unwrap(), 0);

        // Phase 3: append after drain, verify no ghost rows from phase 1 or 2
        store.append_buffer("c", "3").unwrap();
        assert_eq!(store.buffer_count().unwrap(), 1);

        let batch2 = store.drain_buffer().unwrap();
        assert_eq!(batch2.len(), 1, "only the newly appended row should appear");
        assert_eq!(batch2[0], ("c".to_string(), "3".to_string()));
        assert_eq!(store.buffer_count().unwrap(), 0);
    }
}
