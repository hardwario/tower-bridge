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
];

pub fn migrate(conn: &Connection) -> rusqlite::Result<()> {
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
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(version) {
        conn.execute_batch(&format!("BEGIN; {sql}; COMMIT;"))?;
        conn.pragma_update(None, "user_version", (i + 1) as i64)?;
    }
    Ok(())
}
