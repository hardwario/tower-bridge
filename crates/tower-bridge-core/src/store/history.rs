//! Downsampled history for charts.

use rusqlite::{Connection, params};
use serde::Serialize;

use super::Result;

const SECOND: i64 = 1000;
const MINUTE: i64 = 60 * SECOND;
const HOUR: i64 = 60 * MINUTE;

/// Candidate bucket sizes in milliseconds.
const LADDER: &[i64] = &[
    2 * SECOND,
    5 * SECOND,
    10 * SECOND,
    30 * SECOND,
    MINUTE,
    2 * MINUTE,
    5 * MINUTE,
    10 * MINUTE,
    15 * MINUTE,
    30 * MINUTE,
    HOUR,
    2 * HOUR,
    6 * HOUR,
    12 * HOUR,
    24 * HOUR,
];

#[derive(Clone, Debug)]
pub struct HistoryQuery {
    pub from: i64,
    pub to: i64,
    pub max_points: usize,
    /// Sampling interval in ms; a gap longer than 2.5x this (or the bucket)
    /// breaks the line.
    pub sample_interval_ms: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Series {
    pub avg: Vec<Option<f64>>,
    pub min: Vec<Option<f64>>,
    pub max: Vec<Option<f64>>,
}

impl Series {
    fn push(&mut self, avg: Option<f64>, min: Option<f64>, max: Option<f64>) {
        self.avg.push(avg);
        self.min.push(min);
        self.max.push(max);
    }
}

/// Columnar history, ready for uPlot (`t` in seconds).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct History {
    pub from: i64,
    pub to: i64,
    pub bucket_ms: i64,
    pub t: Vec<f64>,
    pub temperature: Series,
    pub humidity: Series,
    pub illuminance: Series,
    pub pressure: Series,
}

/// Smallest ladder step that keeps the point count under `max_points`.
pub fn bucket_for(span_ms: i64, max_points: usize, sample_interval_ms: i64) -> i64 {
    let target = span_ms / max_points.max(1) as i64;
    let floor = sample_interval_ms.max(SECOND);
    LADDER
        .iter()
        .copied()
        .find(|b| *b >= target && *b >= floor)
        .unwrap_or(24 * HOUR)
}

pub fn query(conn: &Connection, q: &HistoryQuery) -> Result<History> {
    let bucket = bucket_for(q.to - q.from, q.max_points, q.sample_interval_ms);
    let mut stmt = conn.prepare_cached(
        "SELECT (ts / ?1) * ?1 AS b,
                AVG(temperature), MIN(temperature), MAX(temperature),
                AVG(humidity), MIN(humidity), MAX(humidity),
                AVG(illuminance), MIN(illuminance), MAX(illuminance),
                AVG(pressure), MIN(pressure), MAX(pressure)
         FROM readings WHERE ts >= ?2 AND ts < ?3
         GROUP BY b ORDER BY b",
    )?;
    let mut h = History {
        from: q.from,
        to: q.to,
        bucket_ms: bucket,
        ..History::default()
    };
    let gap = bucket.max(q.sample_interval_ms) * 5 / 2;
    let mut last: Option<i64> = None;
    let mut rows = stmt.query(params![bucket, q.from, q.to])?;
    while let Some(r) = rows.next()? {
        let b: i64 = r.get(0)?;
        if let Some(prev) = last
            && b - prev > gap
        {
            // Null row so the chart breaks the line.
            let mid = prev + bucket;
            h.t.push(mid as f64 / 1000.0);
            for s in [
                &mut h.temperature,
                &mut h.humidity,
                &mut h.illuminance,
                &mut h.pressure,
            ] {
                s.push(None, None, None);
            }
        }
        last = Some(b);
        // Plot each bucket at its centre.
        h.t.push((b + bucket / 2) as f64 / 1000.0);
        h.temperature.push(r.get(1)?, r.get(2)?, r.get(3)?);
        h.humidity.push(r.get(4)?, r.get(5)?, r.get(6)?);
        h.illuminance.push(r.get(7)?, r.get(8)?, r.get(9)?);
        h.pressure.push(r.get(10)?, r.get(11)?, r.get(12)?);
    }
    Ok(h)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    use crate::store::tests::reading;

    #[test]
    fn bucket_selection() {
        assert_eq!(bucket_for(HOUR, 1000, 10 * SECOND), 10 * SECOND);
        assert_eq!(bucket_for(24 * HOUR, 1000, 10 * SECOND), 2 * MINUTE);
        assert_eq!(bucket_for(30 * 24 * HOUR, 1000, 10 * SECOND), HOUR);
        assert_eq!(bucket_for(HOUR, 1000, 2 * SECOND), 5 * SECOND);
    }

    #[test]
    fn aggregates_and_gaps() -> crate::store::Result<()> {
        let mut s = Store::open_in_memory()?;
        // Two samples per 20 s bucket for one minute, then a 10 minute gap.
        for i in 0..6 {
            s.insert(&reading(i * 10_000, i as f64))?;
        }
        s.insert(&reading(660_000, 100.0))?;
        let h = query(
            s.conn(),
            &HistoryQuery {
                from: 0,
                to: 700_000,
                max_points: 35,
                sample_interval_ms: 10_000,
            },
        )?;
        assert_eq!(h.bucket_ms, 30_000);
        // buckets 0 (0,10,20 s) and 30 s (30,40,50 s), a gap row, then 660 s
        assert_eq!(h.t.len(), 4);
        assert_eq!(h.temperature.avg[0], Some(1.0));
        assert_eq!(h.temperature.min[1], Some(3.0));
        assert_eq!(h.temperature.max[1], Some(5.0));
        assert_eq!(h.temperature.avg[2], None);
        assert_eq!(h.temperature.avg[3], Some(100.0));
        assert_eq!(h.illuminance.avg[0], None);
        Ok(())
    }

    #[test]
    #[ignore = "performance check: cargo test --release -p tower-bridge-core --all-features -- --ignored"]
    fn thirty_days_query_is_fast() -> crate::store::Result<()> {
        let mut s = Store::open_in_memory()?;
        let n = 30 * 8640;
        let rows: Vec<_> = (0..n)
            .map(|i| reading(i * 10_000, 20.0 + (i % 100) as f64 / 10.0))
            .collect();
        s.insert_many(&rows)?;
        let started = std::time::Instant::now();
        let h = query(
            s.conn(),
            &HistoryQuery {
                from: 0,
                to: n * 10_000,
                max_points: 900,
                sample_interval_ms: 10_000,
            },
        )?;
        let took = started.elapsed();
        assert!(h.t.len() <= 900 && h.t.len() > 300, "{} points", h.t.len());
        assert!(took.as_millis() < 150, "query took {took:?}");
        Ok(())
    }
}
