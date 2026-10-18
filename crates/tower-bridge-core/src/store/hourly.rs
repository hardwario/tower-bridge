//! Hourly summary of the readings, for charts of long spans.
//!
//! A year at a 20 s interval is 1.6 million rows per channel; a chart of it
//! needs only about a thousand points. `readings_hourly` keeps, per hour and
//! channel, the sum, count, min and max of every metric, so buckets of an hour
//! or more read a few thousand rows instead. Sum and count rather than the
//! average, so several hours merge into an exact average.
//!
//! The summary of an hour is computed again from its readings whenever a
//! reading of that hour is written or removed, so it never drifts (a replaced
//! reading included). An hour holds at most 1800 readings per channel.

use std::collections::BTreeSet;

use rusqlite::{Connection, params};

use super::Result;

pub const HOUR: i64 = 3_600_000;

/// The start of the hour `ts` falls in.
pub fn hour_of(ts: i64) -> i64 {
    ts.div_euclid(HOUR) * HOUR
}

/// Compute the summary of one hour of one channel again from its readings.
pub fn refresh(conn: &Connection, hour: i64, channel: u8) -> Result<()> {
    conn.prepare_cached("DELETE FROM readings_hourly WHERE hour = ?1 AND channel = ?2")?
        .execute(params![hour, channel])?;
    // No readings left: no row, rather than one of nulls.
    conn.prepare_cached(
        "INSERT INTO readings_hourly
         SELECT ?1, channel,
                SUM(temperature), COUNT(temperature), MIN(temperature), MAX(temperature),
                SUM(humidity), COUNT(humidity), MIN(humidity), MAX(humidity),
                SUM(illuminance), COUNT(illuminance), MIN(illuminance), MAX(illuminance),
                SUM(pressure), COUNT(pressure), MIN(pressure), MAX(pressure),
                SUM(co2), COUNT(co2), MIN(co2), MAX(co2),
                SUM(voc), COUNT(voc), MIN(voc), MAX(voc)
         FROM readings WHERE ts >= ?1 AND ts < ?1 + 3600000 AND channel = ?2
         GROUP BY channel",
    )?
    .execute(params![hour, channel])?;
    Ok(())
}

/// Refresh every hour and channel in `hours` (after a batch of inserts).
pub fn refresh_all(conn: &Connection, hours: &BTreeSet<(i64, u8)>) -> Result<()> {
    for &(hour, channel) in hours {
        refresh(conn, hour, channel)?;
    }
    Ok(())
}

/// Follow a retention purge: drop the hours before `cutoff_ms` and refresh
/// the hour it falls in, which lost the readings before it.
pub fn purge_older_than(conn: &Connection, cutoff_ms: i64) -> Result<()> {
    let hour = hour_of(cutoff_ms);
    conn.execute("DELETE FROM readings_hourly WHERE hour < ?1", params![hour])?;
    let channels: Vec<u8> = conn
        .prepare("SELECT channel FROM readings_hourly WHERE hour = ?1")?
        .query_map(params![hour], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for channel in channels {
        refresh(conn, hour, channel)?;
    }
    Ok(())
}
