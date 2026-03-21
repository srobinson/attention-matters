use crate::error::Result;

use super::Store;

#[derive(Debug)]
pub struct GcResult {
    pub evicted_occurrences: u64,
    pub removed_neighborhoods: u64,
    pub removed_episodes: u64,
    pub before_occurrences: u64,
    pub before_size: u64,
    pub after_size: u64,
}

impl Store {
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

        // Parameters: ?1 = max_epoch_f, ?2 = recency_weight,
        // ?3 = epoch_floor (-1 sentinel disables), ?4 = retention_secs (-1 sentinel disables).
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
}
