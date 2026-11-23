//! SQLite history of readings and alerts.
//!
//! One writer connection (owned by the app's hub thread) inserts readings;
//! readers open short-lived read-only connections. WAL mode lets both run
//! concurrently. Values are stored in SI units (°C, %RH, lux, Pa), one row
//! per measurement cycle and I2C channel in `readings`, and one per cycle
//! and further sensor of a channel (a Humidity Tag next to a Climate Module)
//! in `sensor_readings`.

mod export;
mod history;
mod hourly;
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
use crate::model::{Metric, ModuleKind, Reading, SensorSource};

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

impl StoreError {
    /// The file is damaged or not a database at all.
    pub fn is_corrupt(&self) -> bool {
        matches!(
            self,
            StoreError::Sqlite(rusqlite::Error::SqliteFailure(e, _))
                if matches!(e.code, rusqlite::ErrorCode::NotADatabase | rusqlite::ErrorCode::DatabaseCorrupt)
        )
    }
}

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

    /// Open the database; a damaged one is moved aside (with its journal)
    /// and a new one created, rather than running without history. Returns
    /// where the damaged file went.
    pub fn open_or_replace(path: &Path) -> Result<(Self, Option<std::path::PathBuf>)> {
        match Self::open(path) {
            Err(e) if e.is_corrupt() => {
                let stamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let mut aside = path.as_os_str().to_owned();
                aside.push(format!(".corrupt-{stamp}"));
                let aside = std::path::PathBuf::from(aside);
                std::fs::rename(path, &aside)?;
                // The write-ahead log goes along: it may hold the newest
                // transactions, for whoever tries to rescue the copy.
                let with = |p: &Path, suffix: &str| {
                    let mut name = p.as_os_str().to_owned();
                    name.push(suffix);
                    std::path::PathBuf::from(name)
                };
                let _ = std::fs::rename(with(path, "-wal"), with(&aside, "-wal"));
                let _ = std::fs::remove_file(with(path, "-shm"));
                Ok((Self::open(path)?, Some(aside)))
            }
            other => other.map(|s| (s, None)),
        }
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(mut conn: Connection) -> Result<Self> {
        schema::migrate(&conn)?;
        let tx = conn.transaction()?;
        hourly::catch_up(&tx)?;
        tx.commit()?;
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

    /// Insert a reading, and update the hourly summary of its hour.
    pub fn insert(&mut self, reading: &Reading) -> Result<()> {
        if reading.values.is_empty() {
            return Ok(());
        }
        let device_id = self.device_id(&reading.device, None)?;
        let tx = self.conn.transaction()?;
        insert_row(&tx, device_id, reading)?;
        hourly::refresh(
            &tx,
            hourly::hour_of(reading.ts),
            reading.channel,
            reading.sensor.as_deref(),
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Insert many readings in one transaction; each hour's summary is
    /// computed once at the end.
    pub fn insert_many(&mut self, readings: &[Reading]) -> Result<()> {
        let mut devices = HashMap::new();
        for r in readings.iter().filter(|r| !r.values.is_empty()) {
            if !devices.contains_key(&r.device) {
                devices.insert(r.device.clone(), self.device_id(&r.device, None)?);
            }
        }
        let tx = self.conn.transaction()?;
        let mut hours = std::collections::BTreeSet::new();
        for r in readings {
            if let Some(&device_id) = devices.get(&r.device)
                && !r.values.is_empty()
            {
                insert_row(&tx, device_id, r)?;
                hours.insert((hourly::hour_of(r.ts), r.channel, r.sensor.clone()));
            }
        }
        hourly::refresh_all(&tx, &hours)?;
        // Dropped without a commit on an error: rolled back.
        tx.commit()?;
        Ok(())
    }

    pub fn log_alert(&mut self, ev: &AlertEvent) -> Result<()> {
        self.conn
            .prepare_cached(
                "INSERT INTO alert_log (ts, metric, channel, sensor, bound, kind, value, threshold)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )?
            .execute(params![
                ev.ts,
                ev.metric.as_str(),
                ev.channel,
                ev.sensor,
                bound_str(ev.bound),
                kind_str(ev.kind),
                ev.value,
                ev.threshold
            ])?;
        Ok(())
    }

    /// Record which module carries which sensor from `ts` on, when it
    /// changed since the last record (see [`sources_at`]).
    pub fn log_sources(&mut self, ts: i64, sources: &[SensorSource]) -> Result<()> {
        let last = source_log(&self.conn)?.pop();
        if last.as_ref().is_some_and(|(_, s)| s == &logged(sources)) {
            return Ok(());
        }
        let tx = self.conn.transaction()?;
        if sources.is_empty() {
            // Nothing measured from here on.
            tx.execute(
                "INSERT INTO source_log (ts, channel, metric, sensor, module, is_primary)
                 VALUES (?1, 0, '', '', '', 0)",
                params![ts],
            )?;
        }
        for s in sources {
            tx.prepare_cached(
                "INSERT INTO source_log (ts, channel, metric, sensor, module, is_primary)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?
            .execute(params![
                ts,
                s.channel,
                s.metric.as_str(),
                s.id,
                module_str(s.module),
                s.primary
            ])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Delete data older than `cutoff_ms`; returns deleted reading rows.
    pub fn purge_older_than(&mut self, cutoff_ms: i64) -> Result<usize> {
        purge_older_than(&self.conn, cutoff_ms)
    }
}

fn insert_row(conn: &Connection, device_id: i64, reading: &Reading) -> Result<()> {
    let v = &reading.values;
    if let Some(sensor) = &reading.sensor {
        conn.prepare_cached(
            "INSERT OR REPLACE INTO sensor_readings (ts, device_id, channel, sensor, temperature, humidity, illuminance, pressure, co2, voc)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        )?
        .execute(params![
            reading.ts,
            device_id,
            reading.channel,
            sensor,
            v.temperature,
            v.humidity,
            v.illuminance,
            v.pressure,
            v.co2,
            v.voc
        ])?;
        return Ok(());
    }
    conn.prepare_cached(
        "INSERT OR REPLACE INTO readings (ts, device_id, channel, temperature, humidity, illuminance, pressure, co2, voc)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
    )?
    .execute(params![
        reading.ts,
        device_id,
        reading.channel,
        v.temperature,
        v.humidity,
        v.illuminance,
        v.pressure,
        v.co2,
        v.voc
    ])?;
    Ok(())
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

fn module_str(m: ModuleKind) -> String {
    serde_json::to_value(m)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// A recorded source: channel, metric, sensor id, module, primary.
pub type LoggedSource = (u8, Metric, String, ModuleKind, bool);

fn logged(sources: &[SensorSource]) -> Vec<LoggedSource> {
    sources
        .iter()
        .map(|s| (s.channel, s.metric, s.id.clone(), s.module, s.primary))
        .collect()
}

/// Every record of the sources, oldest first: from its time on, these
/// sources were measured (none after an empty record).
pub fn source_log(conn: &Connection) -> Result<Vec<(i64, Vec<LoggedSource>)>> {
    let mut stmt = conn.prepare_cached(
        "SELECT ts, channel, metric, sensor, module, is_primary FROM source_log ORDER BY ts, rowid",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, u8>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, bool>(5)?,
        ))
    })?;
    let mut out: Vec<(i64, Vec<LoggedSource>)> = Vec::new();
    for row in rows {
        let (ts, channel, metric, sensor, module, primary) = row?;
        if out.last().is_none_or(|(t, _)| *t != ts) {
            out.push((ts, Vec::new()));
        }
        let parsed = (
            serde_json::from_value::<Metric>(serde_json::Value::String(metric)),
            serde_json::from_value::<ModuleKind>(serde_json::Value::String(module)),
        );
        if let (Some((_, list)), (Ok(metric), Ok(module))) = (out.last_mut(), parsed) {
            list.push((channel, metric, sensor, module, primary));
        }
    }
    Ok(out)
}

/// The rows of an export: `ts, serial, temperature, humidity, illuminance,
/// pressure, channel, co2, voc, sensor` (`sensor` empty for a channel's
/// own row), ordered by time, channel and sensor.
pub(crate) const EXPORT_ROWS: &str = "SELECT r.ts, d.serial, r.temperature, r.humidity, r.illuminance, r.pressure, r.channel, r.co2, r.voc, '' AS sensor
         FROM readings r JOIN devices d ON d.id = r.device_id
         WHERE r.ts >= ?1 AND r.ts < ?2
     UNION ALL
     SELECT r.ts, d.serial, r.temperature, r.humidity, r.illuminance, r.pressure, r.channel, r.co2, r.voc, r.sensor
         FROM sensor_readings r JOIN devices d ON d.id = r.device_id
         WHERE r.ts >= ?1 AND r.ts < ?2
     ORDER BY 1, 7, 10";

/// Names the modules behind the rows of an export, from the source log.
pub(crate) struct SourceNames {
    log: Vec<(i64, Vec<LoggedSource>)>,
}

impl SourceNames {
    pub(crate) fn load(conn: &Connection) -> Result<Self> {
        Ok(Self {
            log: source_log(conn)?,
        })
    }

    /// The module of a further `sensor`'s row (`""` for a channel's own),
    /// or the modules of a channel's row, e.g. `Climate Module R2, CO2
    /// Module`; empty when unknown (rows from before the log).
    pub(crate) fn of(&self, ts: i64, channel: u8, sensor: &str) -> String {
        let mut names: Vec<&str> = Vec::new();
        for (c, _, id, module, primary) in sources_at(&self.log, ts) {
            let mine = *c == channel
                && if sensor.is_empty() {
                    *primary
                } else {
                    id == sensor
                };
            if mine && !names.contains(&module.name()) {
                names.push(module.name());
            }
        }
        names.join(", ")
    }
}

/// The sources recorded for the time `ts` (see [`Store::log_sources`]),
/// from records sorted oldest first.
pub fn sources_at(log: &[(i64, Vec<LoggedSource>)], ts: i64) -> &[LoggedSource] {
    let i = log.partition_point(|(t, _)| *t <= ts);
    match i.checked_sub(1).and_then(|i| log.get(i)) {
        Some((_, sources)) => sources,
        None => &[],
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
    /// The further sensor whose series the alert is about; absent for the
    /// channel's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sensor: Option<String>,
    pub bound: String,
    pub kind: String,
    pub value: f64,
    pub threshold: f64,
}

pub fn alert_log(conn: &Connection, limit: usize) -> Result<Vec<AlertLogEntry>> {
    let mut stmt = conn.prepare_cached(
        "SELECT ts, metric, channel, bound, kind, value, threshold, sensor FROM alert_log ORDER BY ts DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit as i64], |r| {
        Ok(AlertLogEntry {
            ts: r.get(0)?,
            metric: r.get(1)?,
            channel: r.get(2)?,
            sensor: r.get(7)?,
            bound: r.get(3)?,
            kind: r.get(4)?,
            value: r.get(5)?,
            threshold: r.get(6)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Number of reading rows (one per channel and cycle, and one per further
/// sensor and cycle) in `[from, to)`.
pub fn count(conn: &Connection, from: i64, to: i64) -> Result<u64> {
    let n: i64 = conn.query_row(
        "SELECT (SELECT COUNT(*) FROM readings WHERE ts >= ?1 AND ts < ?2)
              + (SELECT COUNT(*) FROM sensor_readings WHERE ts >= ?1 AND ts < ?2)",
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
    // Two subqueries: SQLite answers each MIN or MAX from the index, but
    // scans the whole table for both in one.
    let (first, last): (Option<i64>, Option<i64>) = conn.query_row(
        "SELECT (SELECT MIN(ts) FROM readings), (SELECT MAX(ts) FROM readings)",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
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
            sensor: None,
            values: Values {
                temperature: Some(t),
                humidity: Some(40.0),
                illuminance: None,
                pressure: Some(100_000.0),
                co2: None,
                voc: None,
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
            sensor: Some("sht20-40".into()),
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
        assert_eq!(log[0].sensor.as_deref(), Some("sht20-40"));
        Ok(())
    }

    #[test]
    fn a_further_sensor_has_rows_of_its_own() -> Result<()> {
        let mut s = Store::open_in_memory()?;
        let mut tag = reading(1000, 5.0);
        tag.sensor = Some("sht20-40".into());
        s.insert_many(&[reading(1000, 20.0), tag.clone()])?;
        s.insert(&tag)?;
        assert_eq!(
            count(s.conn(), 0, 2000)?,
            2,
            "one row each, the second replaced"
        );
        let t: f64 = s.conn().query_row(
            "SELECT temperature FROM sensor_readings WHERE sensor = 'sht20-40'",
            [],
            |r| r.get(0),
        )?;
        assert_eq!(t, 5.0);
        let n: i64 = s.conn().query_row(
            "SELECT temperature_n FROM sensor_readings_hourly WHERE sensor = 'sht20-40'",
            [],
            |r| r.get(0),
        )?;
        assert_eq!(n, 1, "summarized per hour too");
        Ok(())
    }

    #[test]
    fn sources_are_logged_when_they_change() -> Result<()> {
        let source = |module, id: &str, primary| SensorSource {
            metric: Metric::Humidity,
            channel: 0,
            module,
            sensor: "X".into(),
            address: 0,
            id: id.into(),
            primary,
        };
        let r2 = source(ModuleKind::ClimateModuleR2, "sht30-45", true);
        let tag = source(ModuleKind::HumidityTag, "sht20-40", false);
        let mut s = Store::open_in_memory()?;
        s.log_sources(1000, std::slice::from_ref(&r2))?;
        s.log_sources(2000, std::slice::from_ref(&r2))?;
        s.log_sources(3000, &[r2.clone(), tag])?;
        s.log_sources(4000, &[])?;
        let log = source_log(s.conn())?;
        assert_eq!(
            log.iter().map(|(ts, l)| (*ts, l.len())).collect::<Vec<_>>(),
            vec![(1000, 1), (3000, 2), (4000, 0)],
            "an unchanged set is not logged again"
        );
        assert!(sources_at(&log, 500).is_empty());
        assert_eq!(sources_at(&log, 2500).len(), 1);
        assert_eq!(sources_at(&log, 3000)[1].3, ModuleKind::HumidityTag);
        assert!(sources_at(&log, 9000).is_empty());
        Ok(())
    }

    #[test]
    fn a_damaged_database_is_moved_aside() -> std::result::Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("tower-bridge.db");
        std::fs::write(
            &path,
            b"this is not a database, just text that is long enough",
        )?;
        let (mut store, aside) = Store::open_or_replace(&path)?;
        store.insert(&reading(1000, 21.0))?;
        let Some(aside) = aside else {
            panic!("not moved aside");
        };
        assert!(aside.to_string_lossy().contains(".corrupt-"));
        assert!(std::fs::read(&aside)?.starts_with(b"this is not"));
        // A good database is just opened.
        drop(store);
        let (_, aside) = Store::open_or_replace(&path)?;
        assert!(aside.is_none());
        Ok(())
    }
}
