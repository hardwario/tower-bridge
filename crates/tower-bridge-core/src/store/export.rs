//! CSV export, streamed row by row.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use chrono::{Local, TimeZone};
use rusqlite::{Connection, params};

use super::{Result, StoreError};
use crate::model::{altitude, celsius_to_fahrenheit};

#[derive(Clone, Copy, Debug)]
pub struct CsvOptions {
    pub fahrenheit: bool,
    /// Sea-level reference for the derived altitude column.
    pub sea_level_pa: f64,
}

fn fmt(v: Option<f64>, decimals: usize) -> String {
    v.map(|x| format!("{x:.decimals$}")).unwrap_or_default()
}

/// Write readings in `[from, to)` as CSV. `progress(done, total)` is called
/// every 10 000 rows and may return `false` to cancel.
pub fn write_csv<W: Write>(
    conn: &Connection,
    from: i64,
    to: i64,
    opts: CsvOptions,
    mut out: W,
    mut progress: impl FnMut(u64, u64) -> bool,
) -> Result<u64> {
    let total = super::count(conn, from, to)?;
    let temp_col = if opts.fahrenheit {
        "temperature_f"
    } else {
        "temperature_c"
    };
    writeln!(
        out,
        "timestamp,unix_ms,device,{temp_col},humidity_pct,illuminance_lx,pressure_hpa,altitude_m"
    )?;
    let mut stmt = conn.prepare(
        "SELECT r.ts, d.serial, r.temperature, r.humidity, r.illuminance, r.pressure
         FROM readings r JOIN devices d ON d.id = r.device_id
         WHERE r.ts >= ?1 AND r.ts < ?2 ORDER BY r.ts",
    )?;
    let mut rows = stmt.query(params![from, to])?;
    let mut n = 0u64;
    while let Some(r) = rows.next()? {
        let ts: i64 = r.get(0)?;
        let serial: String = r.get(1)?;
        let t: Option<f64> = r.get(2)?;
        let h: Option<f64> = r.get(3)?;
        let l: Option<f64> = r.get(4)?;
        let p: Option<f64> = r.get(5)?;
        let stamp = Local
            .timestamp_millis_opt(ts)
            .single()
            .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Secs, false))
            .unwrap_or_default();
        let t = if opts.fahrenheit {
            t.map(celsius_to_fahrenheit)
        } else {
            t
        };
        writeln!(
            out,
            "{stamp},{ts},{serial},{},{},{},{},{}",
            fmt(t, 2),
            fmt(h, 1),
            fmt(l, 1),
            fmt(p.map(|p| p / 100.0), 2),
            fmt(p.map(|p| altitude(p, opts.sea_level_pa)), 1),
        )?;
        n += 1;
        if n.is_multiple_of(10_000) && !progress(n, total) {
            return Err(StoreError::Cancelled);
        }
    }
    out.flush()?;
    progress(n, total);
    Ok(n)
}

/// Export into `path` atomically: write `<path>.part`, then rename.
pub fn export_to_file(
    conn: &Connection,
    path: &Path,
    from: i64,
    to: i64,
    opts: CsvOptions,
    progress: impl FnMut(u64, u64) -> bool,
) -> Result<u64> {
    let mut part = path.as_os_str().to_owned();
    part.push(".part");
    let part = std::path::PathBuf::from(part);
    let result = File::create(&part).map_err(StoreError::from).and_then(|f| {
        write_csv(
            conn,
            from,
            to,
            opts,
            BufWriter::with_capacity(64 * 1024, f),
            progress,
        )
    });
    match result {
        Ok(n) => {
            std::fs::rename(&part, path)?;
            Ok(n)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    use crate::store::tests::reading;

    #[test]
    fn csv_layout() -> Result<()> {
        let mut s = Store::open_in_memory()?;
        s.insert(&reading(1_700_000_000_000, 21.456))?;
        let mut buf = Vec::new();
        let n = write_csv(
            s.conn(),
            0,
            i64::MAX,
            CsvOptions {
                fahrenheit: false,
                sea_level_pa: 101_325.0,
            },
            &mut buf,
            |_, _| true,
        )?;
        assert_eq!(n, 1);
        let text = String::from_utf8(buf).map_err(std::io::Error::other)?;
        let mut lines = text.lines();
        assert_eq!(
            lines.next(),
            Some(
                "timestamp,unix_ms,device,temperature_c,humidity_pct,illuminance_lx,pressure_hpa,altitude_m"
            )
        );
        let row: Vec<&str> = lines.next().unwrap_or_default().split(',').collect();
        assert_eq!(row[1], "1700000000000");
        assert_eq!(row[2], "SN1");
        assert_eq!(row[3], "21.46");
        assert_eq!(row[4], "40.0");
        assert_eq!(row[5], "");
        assert_eq!(row[6], "1000.00");
        assert_eq!(row[7], "110.9");
        Ok(())
    }

    #[test]
    fn file_export_is_atomic() -> Result<()> {
        let mut s = Store::open_in_memory()?;
        s.insert(&reading(1000, 20.0))?;
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("out.csv");
        let opts = CsvOptions {
            fahrenheit: true,
            sea_level_pa: 101_325.0,
        };
        export_to_file(s.conn(), &path, 0, i64::MAX, opts, |_, _| true)?;
        let text = std::fs::read_to_string(&path)?;
        assert!(text.contains("temperature_f"));
        assert!(text.contains(",68.00,"));
        assert!(!dir.path().join("out.csv.part").exists());
        Ok(())
    }
}
