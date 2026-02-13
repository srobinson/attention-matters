use std::path::Path;

use rusqlite::{Connection, params};
use uuid::Uuid;

use am_core::{DAESystem, DaemonPhasor, Episode, Neighborhood, Occurrence, Quaternion};

use crate::error::{Result, StoreError};
use crate::schema;

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
        Ok(())
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
}
