//! SQLite history of readings and alerts.
//!
//! One writer connection (owned by the app's hub thread) inserts readings;
//! readers open short-lived read-only connections. WAL mode lets both run
//! concurrently. Values are stored in SI units (°C, %RH, lux, Pa), one row
//! per measurement cycle and I2C channel.

mod export;
mod history;
mod retention;
mod schema;
#[cfg(feature = "xlsx")]
mod xlsx;

use std::collections::HashMap;
use std::path::Path;

pub use rusqlite::Connection;
use rusqlite::{OpenFlags, OptionalExtension, params};
use serde::Serialize;

pub use export::{CsvOptions, export_to_file, write_csv};
pub use history::{History, HistoryQuery, Series, bucket_for, query as history};
pub use retention::purge_older_than;
#[cfg(feature = "xlsx")]
pub use xlsx::{XlsxLabels, export_xlsx_to_file};

use crate::alerts::{AlertEvent, AlertKind, Bound};
use crate::model::Reading;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("cancelled")]
    Cancelled,
    #[error("{0}")]
    Export(String),
}

pub type Result<T> = std::result::Result<T, StoreError>;

/// Read-write handle.
pub struct Store {
    conn: Connection,
    device_ids: HashMap<String, i64>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        Self::from_connection(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(conn: Connection) -> Result<Self> {
        schema::migrate(&conn)?;
        Ok(Self {
            conn,
            device_ids: HashMap::new(),
        })
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Register a device (or update its revision) and return its id.
    pub fn device_id(&mut self, serial: &str, revision: Option<&str>) -> Result<i64> {
        if revision.is_none()
            && let Some(id) = self.device_ids.get(serial)
        {
            return Ok(*id);
        }
        let id: i64 = self.conn.query_row(
            "INSERT INTO devices (serial, revision, first_seen) VALUES (?1, ?2, ?3)
             ON CONFLICT(serial) DO UPDATE SET revision = COALESCE(excluded.revision, devices.revision)
             RETURNING id",
            params![serial, revision, crate::acquisition::unix_ms()],
            |r| r.get(0),
        )?;
        self.device_ids.insert(serial.to_string(), id);
        Ok(id)
    }

    pub fn insert(&mut self, reading: &Reading) -> Result<()> {
        if reading.values.is_empty() {
            return Ok(());
        }
        let device_id = self.device_id(&reading.device, None)?;
        let v = &reading.values;
        self.conn
            .prepare_cached(
                "INSERT OR REPLACE INTO readings (ts, device_id, channel, temperature, humidity, illuminance, pressure)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?
            .execute(params![
                reading.ts,
                device_id,
                reading.channel,
                v.temperature,
                v.humidity,
                v.illuminance,
                v.pressure
            ])?;
        Ok(())
    }

    /// Insert many readings in one transaction.
    pub fn insert_many(&mut self, readings: &[Reading]) -> Result<()> {
        self.conn.execute_batch("BEGIN")?;
        for r in readings {
            if let Err(e) = self.insert(r) {
                let _ = self.conn.execute_batch("ROLLBACK");
                return Err(e);
            }
        }
        self.conn.execute_batch("COMMIT")?;
        Ok(())
    }

    pub fn log_alert(&mut self, ev: &AlertEvent) -> Result<()> {
        self.conn
            .prepare_cached(
                "INSERT INTO alert_log (ts, metric, channel, bound, kind, value, threshold)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?
            .execute(params![
                ev.ts,
                ev.metric.as_str(),
                ev.channel,
                bound_str(ev.bound),
                kind_str(ev.kind),
                ev.value,
                ev.threshold
            ])?;
        Ok(())
    }

    /// Delete data older than `cutoff_ms`; returns deleted reading rows.
    pub fn purge_older_than(&mut self, cutoff_ms: i64) -> Result<usize> {
        purge_older_than(&self.conn, cutoff_ms)
    }
}

fn bound_str(b: Bound) -> &'static str {
    match b {
        Bound::Low => "low",
        Bound::High => "high",
    }
}

fn kind_str(k: AlertKind) -> &'static str {
    match k {
        AlertKind::Triggered => "triggered",
        AlertKind::Recovered => "recovered",
    }
}

/// Open a read-only connection to an existing database.
pub fn open_reader(path: &Path) -> Result<Connection> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    Ok(conn)
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct AlertLogEntry {
    pub ts: i64,
    pub metric: String,
    pub channel: u8,
    pub bound: String,
    pub kind: String,
    pub value: f64,
    pub threshold: f64,
}

pub fn alert_log(conn: &Connection, limit: usize) -> Result<Vec<AlertLogEntry>> {
    let mut stmt = conn.prepare_cached(
        "SELECT ts, metric, channel, bound, kind, value, threshold FROM alert_log ORDER BY ts DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit as i64], |r| {
        Ok(AlertLogEntry {
            ts: r.get(0)?,
            metric: r.get(1)?,
            channel: r.get(2)?,
            bound: r.get(3)?,
            kind: r.get(4)?,
            value: r.get(5)?,
            threshold: r.get(6)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Number of reading rows (one per channel and cycle) in `[from, to)`.
pub fn count(conn: &Connection, from: i64, to: i64) -> Result<u64> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM readings WHERE ts >= ?1 AND ts < ?2",
        params![from, to],
        |r| r.get(0),
    )?;
    Ok(n as u64)
}

/// Timestamp of the oldest reading.
pub fn first_ts(conn: &Connection) -> Result<Option<i64>> {
    Ok(conn
        .query_row("SELECT MIN(ts) FROM readings", [], |r| {
            r.get::<_, Option<i64>>(0)
        })
        .optional()?
        .flatten())
}

/// Oldest and newest reading (Unix ms).
pub fn data_range(conn: &Connection) -> Result<Option<(i64, i64)>> {
    let (first, last): (Option<i64>, Option<i64>) =
        conn.query_row("SELECT MIN(ts), MAX(ts) FROM readings", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?;
    Ok(first.zip(last))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Values;

    pub(crate) fn reading(ts: i64, t: f64) -> Reading {
        Reading {
            ts,
            device: "SN1".into(),
            channel: 0,
            values: Values {
                temperature: Some(t),
                humidity: Some(40.0),
                illuminance: None,
                pressure: Some(100_000.0),
            },
            altitude: None,
        }
    }

    #[test]
    fn insert_and_count() -> Result<()> {
        let mut s = Store::open_in_memory()?;
        for i in 0..10 {
            s.insert(&reading(i * 1000, 20.0 + i as f64))?;
        }
        assert_eq!(count(s.conn(), 0, 10_000)?, 10);
        assert_eq!(count(s.conn(), 5_000, 10_000)?, 5);
        assert_eq!(first_ts(s.conn())?, Some(0));
        assert!(data_range(s.conn())?.is_some_and(|(a, b)| a == 0 && b >= a));
        Ok(())
    }

    #[test]
    fn empty_reading_is_skipped() -> Result<()> {
        let mut s = Store::open_in_memory()?;
        let mut r = reading(0, 1.0);
        r.values = Values::default();
        s.insert(&r)?;
        assert_eq!(count(s.conn(), 0, 1)?, 0);
        Ok(())
    }

    #[test]
    fn device_revision_upsert() -> Result<()> {
        let mut s = Store::open_in_memory()?;
        let a = s.device_id("SN1", Some("R2"))?;
        let b = s.device_id("SN1", None)?;
        let c = s.device_id("SN2", None)?;
        assert_eq!(a, b);
        assert_ne!(a, c);
        Ok(())
    }

    #[test]
    fn alert_log_roundtrip() -> Result<()> {
        let mut s = Store::open_in_memory()?;
        s.log_alert(&AlertEvent {
            ts: 5,
            metric: crate::model::Metric::Temperature,
            channel: 1,
            bound: Bound::High,
            kind: AlertKind::Triggered,
            value: 30.0,
            threshold: 28.0,
            notify: true,
        })?;
        let log = alert_log(s.conn(), 10)?;
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].metric, "temperature");
        assert_eq!(log[0].bound, "high");
        assert_eq!(log[0].channel, 1);
        Ok(())
    }
}
