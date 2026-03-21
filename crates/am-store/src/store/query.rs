use uuid::Uuid;

use am_core::{occurrence::Occurrence, phasor::DaemonPhasor, quaternion::Quaternion};

use crate::error::Result;

use super::{EpisodeInfo, NeighborhoodDetail, NeighborhoodInfo, Store, parse_uuid};

impl Store {
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
}
