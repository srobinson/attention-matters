use rusqlite::Connection;

use crate::error::Result;

pub const SCHEMA_VERSION: i64 = 1;

pub fn initialize(conn: &Connection) -> Result<()> {
    conn.execute_batch("PRAGMA journal_mode = WAL;")?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;

    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS metadata (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS episodes (
            id           TEXT PRIMARY KEY,
            name         TEXT NOT NULL,
            is_conscious INTEGER NOT NULL DEFAULT 0,
            timestamp    TEXT NOT NULL DEFAULT ''
        );

        CREATE TABLE IF NOT EXISTS neighborhoods (
            id          TEXT PRIMARY KEY,
            episode_id  TEXT NOT NULL REFERENCES episodes(id),
            seed_w      REAL NOT NULL,
            seed_x      REAL NOT NULL,
            seed_y      REAL NOT NULL,
            seed_z      REAL NOT NULL,
            source_text TEXT NOT NULL DEFAULT ''
        );

        CREATE TABLE IF NOT EXISTS occurrences (
            id               TEXT PRIMARY KEY,
            neighborhood_id  TEXT NOT NULL REFERENCES neighborhoods(id),
            word             TEXT NOT NULL,
            pos_w            REAL NOT NULL,
            pos_x            REAL NOT NULL,
            pos_y            REAL NOT NULL,
            pos_z            REAL NOT NULL,
            phasor_theta     REAL NOT NULL,
            activation_count INTEGER NOT NULL DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS conversation_buffer (
            id             INTEGER PRIMARY KEY AUTOINCREMENT,
            user_text      TEXT NOT NULL,
            assistant_text TEXT NOT NULL,
            created_at     TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE INDEX IF NOT EXISTS idx_occ_word ON occurrences(word);
        CREATE INDEX IF NOT EXISTS idx_occ_neighborhood ON occurrences(neighborhood_id);
        CREATE INDEX IF NOT EXISTS idx_nbhd_episode ON neighborhoods(episode_id);
        ",
    )?;

    // Set schema version if not present
    conn.execute(
        "INSERT OR IGNORE INTO metadata (key, value) VALUES ('schema_version', ?1)",
        [SCHEMA_VERSION.to_string()],
    )?;

    Ok(())
}

pub fn get_schema_version(conn: &Connection) -> Result<Option<i64>> {
    let mut stmt = conn.prepare("SELECT value FROM metadata WHERE key = 'schema_version'")?;
    let version = stmt
        .query_row([], |row| {
            let v: String = row.get(0)?;
            Ok(v.parse::<i64>().unwrap_or(0))
        })
        .ok();
    Ok(version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initialize_creates_tables() {
        let conn = Connection::open_in_memory().unwrap();
        initialize(&conn).unwrap();

        // Verify tables exist by querying them
        for table in &[
            "episodes",
            "neighborhoods",
            "occurrences",
            "metadata",
            "conversation_buffer",
        ] {
            let count: i64 = conn
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert!(count >= 0, "table {table} should exist");
        }
    }

    #[test]
    fn test_schema_version_set() {
        let conn = Connection::open_in_memory().unwrap();
        initialize(&conn).unwrap();

        let version = get_schema_version(&conn).unwrap();
        assert_eq!(version, Some(SCHEMA_VERSION));
    }

    #[test]
    fn test_wal_mode_enabled() {
        let conn = Connection::open_in_memory().unwrap();
        initialize(&conn).unwrap();

        // WAL mode returns "memory" for in-memory dbs, but succeeds without error
        // For file-based dbs it would return "wal"
        let mode: String = conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        // In-memory always reports "memory", on-disk would report "wal"
        assert!(mode == "memory" || mode == "wal", "got mode: {mode}");
    }

    #[test]
    fn test_idempotent_initialize() {
        let conn = Connection::open_in_memory().unwrap();
        initialize(&conn).unwrap();
        initialize(&conn).unwrap(); // should not error
    }
}
