//! Retention: delete old rows in batches and give pages back to the OS.

use rusqlite::{Connection, OptionalExtension, params};

use super::Result;

const BATCH: i64 = 50_000;

pub fn purge_older_than(conn: &Connection, cutoff_ms: i64) -> Result<usize> {
    let mut total = 0usize;
    loop {
        // Delete in chunks so the writer never holds the lock for long.
        let boundary: Option<i64> = conn
            .query_row(
                "SELECT ts FROM readings WHERE ts < ?1 ORDER BY ts LIMIT 1 OFFSET ?2",
                params![cutoff_ms, BATCH],
                |r| r.get(0),
            )
            .optional()?;
        let upto = boundary.unwrap_or(cutoff_ms);
        let n = conn.execute("DELETE FROM readings WHERE ts < ?1", params![upto])?;
        total += n;
        if boundary.is_none() {
            break;
        }
    }
    conn.execute("DELETE FROM alert_log WHERE ts < ?1", params![cutoff_ms])?;
    conn.execute_batch("PRAGMA incremental_vacuum(2000);")?;
    Ok(total)
}

#[cfg(test)]
mod tests {
    use crate::store::tests::reading;
    use crate::store::{Store, count};

    #[test]
    fn purges_old_rows() -> crate::store::Result<()> {
        let mut s = Store::open_in_memory()?;
        for i in 0..100 {
            s.insert(&reading(i * 1000, 1.0))?;
        }
        assert_eq!(s.purge_older_than(50_000)?, 50);
        assert_eq!(count(s.conn(), 0, 1_000_000)?, 50);
        Ok(())
    }
}
