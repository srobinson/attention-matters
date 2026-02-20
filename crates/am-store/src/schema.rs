use am_core::unix_to_iso8601;
use rusqlite::Connection;

use crate::error::Result;

pub const SCHEMA_VERSION: i64 = 3;

pub fn initialize(conn: &Connection) -> Result<()> {
    conn.execute_batch("PRAGMA journal_mode = WAL;")?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    conn.pragma_update(None, "busy_timeout", 5000)?;
    // Checkpoint every ~400KB instead of the default ~4MB — keeps WAL files small
    conn.pragma_update(None, "wal_autocheckpoint", 100)?;

    // Force-checkpoint any stale WAL data into the main DB on startup.
    // Uses TRUNCATE mode to also remove the WAL file afterward.
    // Errors are non-fatal — in-memory DBs and fresh files legitimately fail this.
    if conn
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .is_ok()
    {
        tracing::info!("startup WAL checkpoint complete");
    }

    // Create tables — for fresh databases this includes project_id.
    // For existing v1 databases, CREATE TABLE IF NOT EXISTS is a no-op,
    // so we ALTER TABLE below to add the missing columns.
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
            timestamp    TEXT NOT NULL DEFAULT '',
            project_id   TEXT NOT NULL DEFAULT ''
        );

        CREATE TABLE IF NOT EXISTS neighborhoods (
            id                 TEXT PRIMARY KEY,
            episode_id         TEXT NOT NULL REFERENCES episodes(id),
            seed_w             REAL NOT NULL,
            seed_x             REAL NOT NULL,
            seed_y             REAL NOT NULL,
            seed_z             REAL NOT NULL,
            source_text        TEXT NOT NULL DEFAULT '',
            neighborhood_type  TEXT NOT NULL DEFAULT 'memory'
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
            created_at     TEXT NOT NULL DEFAULT (datetime('now')),
            project_id     TEXT NOT NULL DEFAULT ''
        );

        CREATE INDEX IF NOT EXISTS idx_occ_word ON occurrences(word);
        CREATE INDEX IF NOT EXISTS idx_occ_neighborhood ON occurrences(neighborhood_id);
        CREATE INDEX IF NOT EXISTS idx_nbhd_episode ON neighborhoods(episode_id);
        ",
    )?;

    // Add project_id to v1 databases that lack it
    if conn
        .prepare("SELECT project_id FROM episodes LIMIT 0")
        .is_err()
    {
        conn.execute_batch(
            "ALTER TABLE episodes ADD COLUMN project_id TEXT NOT NULL DEFAULT '';",
        )?;
    }
    if conn
        .prepare("SELECT project_id FROM conversation_buffer LIMIT 0")
        .is_err()
    {
        conn.execute_batch(
            "ALTER TABLE conversation_buffer ADD COLUMN project_id TEXT NOT NULL DEFAULT '';",
        )?;
    }

    // Add neighborhood_type to v2 databases that lack it
    if conn
        .prepare("SELECT neighborhood_type FROM neighborhoods LIMIT 0")
        .is_err()
    {
        conn.execute_batch(
            "ALTER TABLE neighborhoods ADD COLUMN neighborhood_type TEXT NOT NULL DEFAULT 'memory';",
        )?;
    }

    // Index on project_id (safe to run after ALTER TABLE or on fresh db)
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_ep_project ON episodes(project_id);",
    )?;

    // Backfill empty timestamps on existing episodes using rowid order.
    // Episodes are inserted chronologically, so rowid gives relative ordering.
    // We distribute timestamps from 2026-02-01 to now, spaced evenly.
    backfill_empty_timestamps(conn)?;

    conn.execute(
        "INSERT OR REPLACE INTO metadata (key, value) VALUES ('schema_version', ?1)",
        [SCHEMA_VERSION.to_string()],
    )?;

    Ok(())
}

/// Backfill empty timestamps on episodes using rowid ordering.
/// Only runs once — skips if no episodes have empty timestamps.
fn backfill_empty_timestamps(conn: &Connection) -> Result<()> {
    let empty_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM episodes WHERE timestamp = '' OR timestamp IS NULL",
        [],
        |row| row.get(0),
    )?;

    if empty_count == 0 {
        return Ok(());
    }

    // Get all episodes with empty timestamps, ordered by rowid (insertion order)
    let mut stmt = conn.prepare(
        "SELECT id, rowid FROM episodes WHERE timestamp = '' OR timestamp IS NULL ORDER BY rowid",
    )?;
    let rows: Vec<(String, i64)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<std::result::Result<_, _>>()?;

    if rows.is_empty() {
        return Ok(());
    }

    // Distribute timestamps from 2026-02-01T00:00:00Z to now, evenly spaced
    let now_secs = am_core::now_unix_secs();
    // 2026-02-01T00:00:00Z = 1769904000 Unix seconds
    let start_secs: u64 = 1769904000;
    let end_secs = now_secs.max(start_secs + 1);
    let count = rows.len() as u64;
    let step = (end_secs - start_secs) / count.max(1);

    let tx = conn.unchecked_transaction()?;
    {
        let mut update = tx.prepare(
            "UPDATE episodes SET timestamp = ?1 WHERE id = ?2",
        )?;
        for (i, (id, _rowid)) in rows.iter().enumerate() {
            let ts_secs = start_secs + (i as u64) * step;
            let ts = unix_to_iso8601(ts_secs);
            update.execute(rusqlite::params![ts, id])?;
        }
    }
    tx.commit()?;

    tracing::info!("backfilled timestamps on {count} episodes");
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

    #[test]
    fn test_busy_timeout_set() {
        let conn = Connection::open_in_memory().unwrap();
        initialize(&conn).unwrap();

        let timeout: i64 = conn
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .unwrap();
        assert_eq!(timeout, 5000, "busy_timeout should be 5000ms");
    }

    #[test]
    fn test_wal_autocheckpoint_set() {
        let conn = Connection::open_in_memory().unwrap();
        initialize(&conn).unwrap();

        let threshold: i64 = conn
            .query_row("PRAGMA wal_autocheckpoint", [], |row| row.get(0))
            .unwrap();
        assert_eq!(threshold, 100, "wal_autocheckpoint should be 100 pages");
    }

    #[test]
    fn test_upgrade_v1_to_v3_adds_project_id_and_neighborhood_type() {
        let conn = Connection::open_in_memory().unwrap();

        // Simulate v1 schema: no project_id columns, no neighborhood_type
        conn.execute_batch(
            "
            CREATE TABLE metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            INSERT INTO metadata (key, value) VALUES ('schema_version', '1');

            CREATE TABLE episodes (
                id           TEXT PRIMARY KEY,
                name         TEXT NOT NULL,
                is_conscious INTEGER NOT NULL DEFAULT 0,
                timestamp    TEXT NOT NULL DEFAULT ''
            );
            CREATE TABLE neighborhoods (
                id          TEXT PRIMARY KEY,
                episode_id  TEXT NOT NULL REFERENCES episodes(id),
                seed_w REAL NOT NULL, seed_x REAL NOT NULL,
                seed_y REAL NOT NULL, seed_z REAL NOT NULL,
                source_text TEXT NOT NULL DEFAULT ''
            );
            CREATE TABLE occurrences (
                id TEXT PRIMARY KEY,
                neighborhood_id TEXT NOT NULL REFERENCES neighborhoods(id),
                word TEXT NOT NULL,
                pos_w REAL NOT NULL, pos_x REAL NOT NULL,
                pos_y REAL NOT NULL, pos_z REAL NOT NULL,
                phasor_theta REAL NOT NULL,
                activation_count INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE conversation_buffer (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                user_text TEXT NOT NULL,
                assistant_text TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            INSERT INTO episodes (id, name, is_conscious) VALUES ('ep1', 'test', 0);
            ",
        )
        .unwrap();

        // Run initialize — should upgrade v1 → v2
        initialize(&conn).unwrap();

        // project_id column should exist and default to ''
        let pid: String = conn
            .query_row(
                "SELECT COALESCE(project_id, '') FROM episodes WHERE id = 'ep1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(pid, "");

        // conversation_buffer should also have project_id
        conn.execute(
            "INSERT INTO conversation_buffer (user_text, assistant_text, project_id) VALUES ('u', 'a', 'test')",
            [],
        )
        .unwrap();

        let version = get_schema_version(&conn).unwrap();
        assert_eq!(version, Some(3));

        // neighborhood_type column should exist after v2→v3 migration
        conn.execute_batch(
            "INSERT INTO neighborhoods (id, episode_id, source_text, seed_w, seed_x, seed_y, seed_z, neighborhood_type) \
             VALUES ('n1', 'ep1', 'test', 1.0, 0.0, 0.0, 0.0, 'decision');",
        )
        .unwrap();
        let nbhd_type: String = conn
            .query_row(
                "SELECT neighborhood_type FROM neighborhoods WHERE id = 'n1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(nbhd_type, "decision");
    }
}
