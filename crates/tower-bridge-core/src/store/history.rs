//! Downsampled history for charts.

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

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

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Series {
    pub avg: Vec<Option<f64>>,
    pub min: Vec<Option<f64>>,
    pub max: Vec<Option<f64>>,
}

impl Series {
    fn with_len(n: usize) -> Self {
        Self {
            avg: vec![None; n],
            min: vec![None; n],
            max: vec![None; n],
        }
    }

    fn set(&mut self, i: usize, avg: Option<f64>, min: Option<f64>, max: Option<f64>) {
        self.avg[i] = avg;
        self.min[i] = min;
        self.max[i] = max;
    }
}

/// Columnar history, ready for uPlot (`t` in seconds). Each metric has one
/// series per entry of `channels`, all aligned with `t`; a channel without a
/// value in a bucket has `null` there.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct History {
    pub from: i64,
    pub to: i64,
    pub bucket_ms: i64,
    pub t: Vec<f64>,
    /// I2C channels with readings in the range, ascending.
    pub channels: Vec<u8>,
    pub temperature: Vec<Series>,
    pub humidity: Vec<Series>,
    pub illuminance: Vec<Series>,
    pub pressure: Vec<Series>,
    pub co2: Vec<Series>,
    pub voc: Vec<Series>,
}

impl History {
    /// The series of `metric` measured on `channel`.
    pub fn series(&self, metric: crate::model::Metric, channel: u8) -> Option<&Series> {
        use crate::model::Metric;
        let i = self.channels.iter().position(|c| *c == channel)?;
        match metric {
            Metric::Temperature => self.temperature.get(i),
            Metric::Humidity => self.humidity.get(i),
            Metric::Illuminance => self.illuminance.get(i),
            Metric::Pressure => self.pressure.get(i),
            Metric::Co2 => self.co2.get(i),
            Metric::Voc => self.voc.get(i),
        }
    }
}

/// Metrics in the query, in `Metric::ALL` order.
const METRICS: usize = crate::model::Metric::ALL.len();

/// One aggregated row of the query: a bucket of one channel.
struct Row {
    bucket: i64,
    channel: u8,
    /// avg, min, max per metric.
    values: [[Option<f64>; 3]; METRICS],
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

/// Buckets of the readings themselves.
const FROM_READINGS: &str = "SELECT (ts / ?1) * ?1 AS b, channel,
        AVG(temperature), MIN(temperature), MAX(temperature),
        AVG(humidity), MIN(humidity), MAX(humidity),
        AVG(illuminance), MIN(illuminance), MAX(illuminance),
        AVG(pressure), MIN(pressure), MAX(pressure),
        AVG(co2), MIN(co2), MAX(co2),
        AVG(voc), MIN(voc), MAX(voc)
    FROM readings WHERE ts >= ?2 AND ts < ?3
    GROUP BY b, channel ORDER BY b, channel";

/// Buckets of hours: the whole hours inside the range from the hourly
/// summary, the parts of the first and last hour from the readings, so a
/// bucket holds exactly the readings of `[from, to)` as with
/// `FROM_READINGS`. `?4` is the first whole hour, `?5` the end of the last.
const FROM_HOURS: &str = "SELECT (hour / ?1) * ?1 AS b, channel,
        SUM(temperature_sum) / SUM(temperature_n), MIN(temperature_min), MAX(temperature_max),
        SUM(humidity_sum) / SUM(humidity_n), MIN(humidity_min), MAX(humidity_max),
        SUM(illuminance_sum) / SUM(illuminance_n), MIN(illuminance_min), MAX(illuminance_max),
        SUM(pressure_sum) / SUM(pressure_n), MIN(pressure_min), MAX(pressure_max),
        SUM(co2_sum) / SUM(co2_n), MIN(co2_min), MAX(co2_max),
        SUM(voc_sum) / SUM(voc_n), MIN(voc_min), MAX(voc_max)
    FROM (
        SELECT hour, channel, temperature_sum, temperature_n, temperature_min, temperature_max, humidity_sum, humidity_n, humidity_min, humidity_max, illuminance_sum, illuminance_n, illuminance_min, illuminance_max, pressure_sum, pressure_n, pressure_min, pressure_max, co2_sum, co2_n, co2_min, co2_max, voc_sum, voc_n, voc_min, voc_max
            FROM readings_hourly WHERE hour >= ?4 AND hour < ?5
        UNION ALL
        SELECT (ts / 3600000) * 3600000, channel,
                temperature, temperature IS NOT NULL, temperature, temperature,
                humidity, humidity IS NOT NULL, humidity, humidity,
                illuminance, illuminance IS NOT NULL, illuminance, illuminance,
                pressure, pressure IS NOT NULL, pressure, pressure,
                co2, co2 IS NOT NULL, co2, co2,
                voc, voc IS NOT NULL, voc, voc
            FROM readings WHERE ts >= ?2 AND ts < ?3 AND (ts < ?4 OR ts >= ?5)
    )
    GROUP BY b, channel ORDER BY b, channel";

/// The aggregated rows of `[from, to)` in buckets of `bucket` ms.
fn rows(conn: &Connection, bucket: i64, from: i64, to: i64, hours: bool) -> Result<Vec<Row>> {
    let mut stmt = conn.prepare_cached(if hours { FROM_HOURS } else { FROM_READINGS })?;
    let floor = from.div_euclid(HOUR) * HOUR;
    let first_whole = if floor == from {
        from
    } else {
        floor.saturating_add(HOUR)
    };
    let whole_end = to.div_euclid(HOUR) * HOUR;
    let map = |r: &rusqlite::Row<'_>| {
        let mut values = [[None; 3]; METRICS];
        for (m, v) in values.iter_mut().enumerate() {
            for (k, x) in v.iter_mut().enumerate() {
                *x = r.get(2 + m * 3 + k)?;
            }
        }
        Ok(Row {
            bucket: r.get(0)?,
            channel: r.get(1)?,
            values,
        })
    };
    let rows = if hours {
        stmt.query_map(params![bucket, from, to, first_whole, whole_end], map)?
            .collect::<rusqlite::Result<_>>()?
    } else {
        stmt.query_map(params![bucket, from, to], map)?
            .collect::<rusqlite::Result<_>>()?
    };
    Ok(rows)
}

pub fn query(conn: &Connection, q: &HistoryQuery) -> Result<History> {
    let bucket = bucket_for(
        q.to.saturating_sub(q.from),
        q.max_points,
        q.sample_interval_ms,
    );
    // Whole hours come from the summary: a year is 9000 rows per channel
    // there instead of 1.6 million.
    let hours = bucket % HOUR == 0;
    let rows = rows(conn, bucket, q.from, q.to, hours)?;

    let mut channels: Vec<u8> = rows.iter().map(|r| r.channel).collect();
    channels.sort_unstable();
    channels.dedup();

    // Bucket starts, with a null row wherever data is missing for longer than
    // a few samples, so the chart breaks the line.
    let gap = bucket.max(q.sample_interval_ms) * 5 / 2;
    let mut starts: Vec<i64> = Vec::new();
    let mut nulls: Vec<i64> = Vec::new();
    for r in &rows {
        match starts.last() {
            Some(&prev) if prev == r.bucket => continue,
            Some(&prev) if r.bucket - prev > gap => nulls.push(prev + bucket),
            _ => {}
        }
        starts.push(r.bucket);
    }
    let mut t: Vec<(i64, bool)> = starts
        .iter()
        .map(|b| (*b, true))
        .chain(nulls.iter().map(|b| (*b, false)))
        .collect();
    t.sort_unstable();

    let n = t.len();
    let empty = || vec![Series::with_len(n); channels.len()];
    let mut h = History {
        from: q.from,
        to: q.to,
        bucket_ms: bucket,
        // Plot each bucket at its centre.
        t: t.iter()
            .map(|(b, _)| (b + bucket / 2) as f64 / 1000.0)
            .collect(),
        channels: channels.clone(),
        temperature: empty(),
        humidity: empty(),
        illuminance: empty(),
        pressure: empty(),
        co2: empty(),
        voc: empty(),
    };
    let mut i = 0;
    for r in &rows {
        while i < n && !(t[i].0 == r.bucket && t[i].1) {
            i += 1;
        }
        let Some(c) = channels.iter().position(|c| *c == r.channel) else {
            continue;
        };
        for (m, series) in [
            &mut h.temperature,
            &mut h.humidity,
            &mut h.illuminance,
            &mut h.pressure,
            &mut h.co2,
            &mut h.voc,
        ]
        .into_iter()
        .enumerate()
        {
            let [avg, min, max] = r.values[m];
            if i < n {
                series[c].set(i, avg, min, max);
            }
        }
    }
    Ok(h)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    use crate::store::tests::reading;
    use rusqlite::Connection;

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
        assert_eq!(h.channels, vec![0]);
        let t = &h.temperature[0];
        assert_eq!(t.avg[0], Some(1.0));
        assert_eq!(t.min[1], Some(3.0));
        assert_eq!(t.max[1], Some(5.0));
        assert_eq!(t.avg[2], None);
        assert_eq!(t.avg[3], Some(100.0));
        assert_eq!(h.illuminance[0].avg[0], None);
        Ok(())
    }

    #[test]
    fn one_series_per_channel_on_shared_buckets() -> crate::store::Result<()> {
        let mut s = Store::open_in_memory()?;
        for i in 0..6 {
            s.insert(&reading(i * 10_000, 20.0))?;
            // Channel 1 starts later.
            if i >= 3 {
                let mut r = reading(i * 10_000, 10.0);
                r.channel = 1;
                s.insert(&r)?;
            }
        }
        let h = query(
            s.conn(),
            &HistoryQuery {
                from: 0,
                to: 60_000,
                max_points: 2,
                sample_interval_ms: 10_000,
            },
        )?;
        assert_eq!(h.channels, vec![0, 1]);
        assert_eq!(h.t.len(), 2);
        assert_eq!(h.temperature[0].avg, vec![Some(20.0), Some(20.0)]);
        assert_eq!(h.temperature[1].avg, vec![None, Some(10.0)]);
        assert_eq!(
            h.series(crate::model::Metric::Temperature, 1)
                .map(|s| s.avg[1]),
            Some(Some(10.0))
        );
        Ok(())
    }

    /// Both sources agree, bucket by bucket, within rounding.
    fn assert_same(conn: &Connection, bucket: i64, from: i64, to: i64) -> crate::store::Result<()> {
        let raw = rows(conn, bucket, from, to, false)?;
        let summary = rows(conn, bucket, from, to, true)?;
        assert_eq!(raw.len(), summary.len(), "bucket count");
        for (a, b) in raw.iter().zip(&summary) {
            assert_eq!((a.bucket, a.channel), (b.bucket, b.channel));
            for (m, (x, y)) in a.values.iter().zip(&b.values).enumerate() {
                for k in 0..3 {
                    match (x[k], y[k]) {
                        (Some(x), Some(y)) => {
                            assert!((x - y).abs() < 1e-9, "metric {m}: {x} vs {y}")
                        }
                        (x, y) => assert_eq!(x, y, "metric {m} at {}", a.bucket),
                    }
                }
            }
        }
        Ok(())
    }

    /// Three days every 7 minutes on two channels, with gaps in the values.
    fn three_days(s: &mut Store) -> crate::store::Result<()> {
        let rows: Vec<_> = (0..3 * 24 * 60 / 7)
            .flat_map(|i| {
                [0u8, 1].map(|channel| {
                    let mut r = reading(i * 7 * MINUTE, (i % 37) as f64 * 0.5 + channel as f64);
                    r.channel = channel;
                    r.values.co2 = (i % 3 == 0).then_some(400.0 + i as f64);
                    r.values.humidity = None;
                    r
                })
            })
            .collect();
        s.insert_many(&rows)
    }

    #[test]
    fn the_hourly_summary_matches_the_readings() -> crate::store::Result<()> {
        let mut s = Store::open_in_memory()?;
        three_days(&mut s)?;
        for bucket in [HOUR, 2 * HOUR, 6 * HOUR, 24 * HOUR] {
            assert_same(s.conn(), bucket, 0, 3 * 24 * HOUR)?;
        }
        // Ranges off the hour, as in another time zone or a custom range:
        // the edges hold only the readings inside.
        for (from, to) in [
            (25 * HOUR + 20 * MINUTE, 60 * HOUR + 10 * MINUTE),
            (30 * MINUTE, 50 * MINUTE),
        ] {
            for bucket in [HOUR, 2 * HOUR, 6 * HOUR] {
                assert_same(s.conn(), bucket, from, to)?;
            }
        }
        // Single inserts keep it current too, also replacing a reading.
        s.insert(&reading(3 * 24 * HOUR + 60_000, 50.0))?;
        s.insert(&reading(3 * 24 * HOUR + 120_000, 10.0))?;
        s.insert(&reading(3 * 24 * HOUR + 60_000, 30.0))?;
        assert_same(s.conn(), HOUR, 0, 4 * 24 * HOUR)?;
        let last = rows(s.conn(), HOUR, 3 * 24 * HOUR, 4 * 24 * HOUR, true)?;
        assert_eq!(last[0].values[0], [Some(20.0), Some(10.0), Some(30.0)]);
        // A retention cutoff in the middle of an hour.
        s.purge_older_than(25 * HOUR + 20 * MINUTE)?;
        assert_same(s.conn(), HOUR, 0, 4 * 24 * HOUR)?;
        let first = rows(s.conn(), HOUR, 0, 4 * 24 * HOUR, true)?;
        assert_eq!(first[0].bucket, 25 * HOUR);
        Ok(())
    }

    #[test]
    fn hours_written_by_an_older_build_are_summarized_on_open()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("t.db");
        {
            let mut s = Store::open(&path)?;
            three_days(&mut s)?;
        }
        // An older build knows nothing of the summary and writes readings only.
        {
            let conn = Connection::open(&path)?;
            for i in 0..120 {
                conn.execute(
                    "INSERT INTO readings (ts, device_id, channel, temperature) VALUES (?1, 1, 0, ?2)",
                    rusqlite::params![3 * 24 * HOUR + i * MINUTE, 30.0 + i as f64 / 10.0],
                )?;
            }
        }
        let s = Store::open(&path)?;
        assert_same(s.conn(), HOUR, 0, 4 * 24 * HOUR)?;
        let hours = rows(s.conn(), HOUR, 3 * 24 * HOUR, 4 * 24 * HOUR, true)?;
        assert_eq!(hours.len(), 2, "both hours the older build wrote");
        Ok(())
    }

    #[test]
    fn long_spans_read_the_hourly_summary() -> crate::store::Result<()> {
        let mut s = Store::open_in_memory()?;
        three_days(&mut s)?;
        // Readings the summary does not know about show which source was read.
        s.conn().execute(
            "INSERT INTO readings (ts, device_id, channel, temperature) VALUES (60000, 1, 0, -100.0)",
            [],
        )?;
        let q = |to: i64| HistoryQuery {
            from: 0,
            to,
            max_points: 40,
            sample_interval_ms: 20_000,
        };
        let long = query(s.conn(), &q(3 * 24 * HOUR))?;
        assert_eq!(long.bucket_ms, 2 * HOUR);
        assert_ne!(long.temperature[0].min[0], Some(-100.0), "from the summary");
        let short = query(s.conn(), &q(HOUR))?;
        assert_eq!(short.bucket_ms, 2 * MINUTE);
        assert_eq!(
            short.temperature[0].min[0],
            Some(-100.0),
            "from the readings"
        );
        Ok(())
    }

    /// 30 days every 10 s on `channels`; returns the history query's duration.
    fn thirty_days(channels: &[u8]) -> crate::store::Result<std::time::Duration> {
        let mut s = Store::open_in_memory()?;
        let n = 30 * 8640;
        let rows: Vec<_> = (0..n)
            .flat_map(|i| {
                channels.iter().map(move |&channel| {
                    let mut r = reading(i * 10_000, 20.0 + (i % 100) as f64 / 10.0);
                    r.channel = channel;
                    r
                })
            })
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
        assert_eq!(h.channels, channels);
        Ok(took)
    }

    #[test]
    #[ignore = "performance check: cargo test --release -p tower-bridge-core --all-features -- --ignored"]
    fn thirty_days_query_is_fast() -> crate::store::Result<()> {
        let one = thirty_days(&[0])?;
        assert!(one.as_millis() < 150, "one channel took {one:?}");
        // Twice the rows: the time should grow no more than in proportion.
        let two = thirty_days(&[0, 1])?;
        assert!(two.as_millis() < 300, "two channels took {two:?}");
        eprintln!("30 days: one channel {one:?}, two channels {two:?}");
        Ok(())
    }
}
