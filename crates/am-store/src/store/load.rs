use uuid::Uuid;

use am_core::{
    episode::Episode,
    neighborhood::{Neighborhood, NeighborhoodType},
    occurrence::Occurrence,
    phasor::DaemonPhasor,
    quaternion::Quaternion,
    system::DAESystem,
};

use crate::error::Result;

use super::{Store, parse_uuid};

impl Store {
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
}
