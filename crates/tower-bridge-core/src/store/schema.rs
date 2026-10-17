//! Schema migrations (`PRAGMA user_version` ladder).

use rusqlite::Connection;

const MIGRATIONS: &[&str] = &[
    // 1
    "CREATE TABLE devices (
        id INTEGER PRIMARY KEY,
        serial TEXT NOT NULL UNIQUE,
        revision TEXT,
        first_seen INTEGER NOT NULL
    );
    CREATE TABLE readings (
        ts INTEGER NOT NULL,
        device_id INTEGER NOT NULL REFERENCES devices(id),
        temperature REAL,
        humidity REAL,
        illuminance REAL,
        pressure REAL,
        PRIMARY KEY (ts, device_id)
    ) WITHOUT ROWID;
    CREATE TABLE alert_log (
        ts INTEGER NOT NULL,
        metric TEXT NOT NULL,
        bound TEXT NOT NULL,
        kind TEXT NOT NULL,
        value REAL NOT NULL,
        threshold REAL NOT NULL
    );
    CREATE INDEX idx_alert_log_ts ON alert_log(ts);",
    // 2: readings from several I2C channels. Earlier rows come from the
    // Climate Module, recorded as channel 0.
    "CREATE TABLE readings_v2 (
        ts INTEGER NOT NULL,
        device_id INTEGER NOT NULL REFERENCES devices(id),
        channel INTEGER NOT NULL DEFAULT 0,
        temperature REAL,
        humidity REAL,
        illuminance REAL,
        pressure REAL,
        PRIMARY KEY (ts, device_id, channel)
    ) WITHOUT ROWID;
    INSERT INTO readings_v2 (ts, device_id, channel, temperature, humidity, illuminance, pressure)
        SELECT ts, device_id, 0, temperature, humidity, illuminance, pressure FROM readings;
    DROP TABLE readings;
    ALTER TABLE readings_v2 RENAME TO readings;
    ALTER TABLE alert_log ADD COLUMN channel INTEGER NOT NULL DEFAULT 0;",
    // 3: CO2 (ppm) and TVOC (ppb).
    "ALTER TABLE readings ADD COLUMN co2 REAL;
    ALTER TABLE readings ADD COLUMN voc REAL;",
];

pub fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    migrate_to(conn, MIGRATIONS.len())
}

/// Apply the migrations up to `target` (tests start from older versions).
fn migrate_to(conn: &Connection, target: usize) -> rusqlite::Result<()> {
    apply(conn, MIGRATIONS, target)
}

fn apply(conn: &Connection, migrations: &[&str], target: usize) -> rusqlite::Result<()> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    let version = usize::try_from(version).unwrap_or(0);
    if version == 0 {
        // Must be set before the first table is created.
        conn.pragma_update(None, "auto_vacuum", "INCREMENTAL")?;
    }
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    for (i, sql) in migrations.iter().enumerate().take(target).skip(version) {
        // The version changes in the same transaction as the schema, so a
        // crash can never leave a migrated schema with the old version.
        let batch = format!("BEGIN; {sql}; PRAGMA user_version = {}; COMMIT;", i + 1);
        if let Err(e) = conn.execute_batch(&batch) {
            if !conn.is_autocommit() {
                let _ = conn.execute_batch("ROLLBACK");
            }
            return Err(e);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(conn: &Connection) -> rusqlite::Result<i64> {
        conn.pragma_query_value(None, "user_version", |r| r.get(0))
    }

    #[test]
    fn a_new_database_ends_at_the_latest_version() -> rusqlite::Result<()> {
        let conn = Connection::open_in_memory()?;
        migrate(&conn)?;
        assert_eq!(version(&conn)?, MIGRATIONS.len() as i64);
        migrate(&conn)?;
        assert_eq!(
            version(&conn)?,
            MIGRATIONS.len() as i64,
            "a no-op the second time"
        );
        Ok(())
    }

    #[test]
    fn a_failed_migration_changes_nothing() -> rusqlite::Result<()> {
        let conn = Connection::open_in_memory()?;
        let migrations = [
            "CREATE TABLE a (x INTEGER);",
            "CREATE TABLE b (x INTEGER); INSERT INTO nowhere VALUES (1);",
        ];
        assert!(apply(&conn, &migrations, 2).is_err());
        assert_eq!(version(&conn)?, 1);
        assert!(conn.is_autocommit(), "no transaction left open");
        let b: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name = 'b'",
            [],
            |r| r.get(0),
        )?;
        assert_eq!(b, 0, "the half-done migration was rolled back");
        Ok(())
    }

    #[test]
    fn version_1_readings_become_channel_0() -> rusqlite::Result<()> {
        let conn = Connection::open_in_memory()?;
        migrate_to(&conn, 1)?;
        conn.execute_batch(
            "INSERT INTO devices (id, serial, first_seen) VALUES (1, 'SN1', 0);
             INSERT INTO readings VALUES (1000, 1, 21.5, 40.0, 300.0, 100000.0);
             INSERT INTO alert_log VALUES (1000, 'temperature', 'high', 'triggered', 30.0, 28.0);",
        )?;
        migrate(&conn)?;
        let (channel, t): (i64, f64) = conn.query_row(
            "SELECT channel, temperature FROM readings WHERE ts = 1000",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        assert_eq!((channel, t), (0, 21.5));
        let channel: i64 = conn.query_row("SELECT channel FROM alert_log", [], |r| r.get(0))?;
        assert_eq!(channel, 0);
        // The same moment may now have a row per channel.
        conn.execute(
            "INSERT INTO readings (ts, device_id, channel, temperature) VALUES (1000, 1, 1, 12.0)",
            [],
        )?;
        Ok(())
    }
}
