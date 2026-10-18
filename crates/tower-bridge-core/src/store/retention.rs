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
    super::hourly::purge_older_than(conn, cutoff_ms)?;
    conn.execute("DELETE FROM alert_log WHERE ts < ?1", params![cutoff_ms])?;
    // Each step of this pragma frees one page, and `execute_batch` steps
    // only once: step it to the end.
    let mut vacuum = conn.prepare("PRAGMA incremental_vacuum")?;
    let mut rows = vacuum.query([])?;
    while rows.next()?.is_some() {}
    Ok(total)
}

#[cfg(test)]
mod tests {
    use crate::store::tests::reading;
    use crate::store::{Store, count};

    #[test]
    fn gives_the_freed_pages_back() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let mut s = Store::open(&dir.path().join("t.db"))?;
        let rows: Vec<_> = (0..20_000).map(|i| reading(i * 1000, 1.0)).collect();
        s.insert_many(&rows)?;
        let pages = |s: &Store| -> rusqlite::Result<i64> {
            s.conn().query_row("PRAGMA page_count", [], |r| r.get(0))
        };
        let before = pages(&s)?;
        s.purge_older_than(19_000_000)?;
        let free: i64 = s
            .conn()
            .query_row("PRAGMA freelist_count", [], |r| r.get(0))?;
        assert_eq!(free, 0, "no free pages left in the file");
        assert!(pages(&s)? < before / 2, "{} of {before} pages", pages(&s)?);
        Ok(())
    }

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
