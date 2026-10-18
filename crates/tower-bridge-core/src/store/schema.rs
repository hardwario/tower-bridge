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
    // 4: an hourly summary for charts of long spans (see hourly.rs), built
    // from the readings so far.
    "CREATE TABLE readings_hourly (
        hour INTEGER NOT NULL,
        channel INTEGER NOT NULL,
        temperature_sum REAL, temperature_n INTEGER NOT NULL, temperature_min REAL, temperature_max REAL,
        humidity_sum REAL, humidity_n INTEGER NOT NULL, humidity_min REAL, humidity_max REAL,
        illuminance_sum REAL, illuminance_n INTEGER NOT NULL, illuminance_min REAL, illuminance_max REAL,
        pressure_sum REAL, pressure_n INTEGER NOT NULL, pressure_min REAL, pressure_max REAL,
        co2_sum REAL, co2_n INTEGER NOT NULL, co2_min REAL, co2_max REAL,
        voc_sum REAL, voc_n INTEGER NOT NULL, voc_min REAL, voc_max REAL,
        PRIMARY KEY (hour, channel)
    ) WITHOUT ROWID;
    INSERT INTO readings_hourly
        SELECT (ts / 3600000) * 3600000, channel,
               SUM(temperature), COUNT(temperature), MIN(temperature), MAX(temperature),
               SUM(humidity), COUNT(humidity), MIN(humidity), MAX(humidity),
               SUM(illuminance), COUNT(illuminance), MIN(illuminance), MAX(illuminance),
               SUM(pressure), COUNT(pressure), MIN(pressure), MAX(pressure),
               SUM(co2), COUNT(co2), MIN(co2), MAX(co2),
               SUM(voc), COUNT(voc), MIN(voc), MAX(voc)
        FROM readings GROUP BY 1, 2;",
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
    fn version_3_readings_get_an_hourly_summary() -> rusqlite::Result<()> {
        let conn = Connection::open_in_memory()?;
        migrate_to(&conn, 3)?;
        conn.execute_batch(
            "INSERT INTO devices (id, serial, first_seen) VALUES (1, 'SN1', 0);
             INSERT INTO readings (ts, device_id, channel, temperature, co2) VALUES
                 (3600000, 1, 0, 20.0, NULL),
                 (3610000, 1, 0, 22.0, 800.0),
                 (7200000, 1, 0, 30.0, NULL),
                 (3600000, 1, 1, 10.0, NULL);",
        )?;
        migrate(&conn)?;
        let rows: Vec<(i64, i64, f64, i64, f64, f64, i64)> = conn
            .prepare(
                "SELECT hour, channel, temperature_sum, temperature_n, temperature_min,
                        temperature_max, co2_n
                 FROM readings_hourly ORDER BY hour, channel",
            )?
            .query_map([], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ))
            })?
            .collect::<rusqlite::Result<_>>()?;
        assert_eq!(
            rows,
            vec![
                (3_600_000, 0, 42.0, 2, 20.0, 22.0, 1),
                (3_600_000, 1, 10.0, 1, 10.0, 10.0, 0),
                (7_200_000, 0, 30.0, 1, 30.0, 30.0, 0),
            ]
        );
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
