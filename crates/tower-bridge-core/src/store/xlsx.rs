//! Excel (XLSX) export, streamed in constant memory.

use std::path::Path;

use chrono::{Datelike, Local, TimeZone, Timelike};
use rusqlite::{Connection, params};
use rust_xlsxwriter::{ExcelDateTime, Format, FormatBorder, Workbook, Worksheet, XlsxError};

use super::{CsvOptions, Result, StoreError};
use crate::model::{altitude, celsius_to_fahrenheit};

/// Excel allows 1 048 576 rows per sheet, one of them is the header.
const MAX_DATA_ROWS: u32 = 1_048_575;

/// Localised sheet name and column headers (with units).
#[derive(Clone, Debug)]
pub struct XlsxLabels {
    /// Excel number format of the time column, e.g. `yyyy-mm-dd hh:mm:ss`.
    pub datetime_format: String,
    pub sheet: String,
    pub time: String,
    pub device: String,
    pub channel: String,
    pub temperature: String,
    pub humidity: String,
    pub illuminance: String,
    pub pressure: String,
    pub altitude: String,
}

fn xlsx_err(e: XlsxError) -> StoreError {
    StoreError::Export(e.to_string())
}

/// Sheet names: at most 31 characters, without `[]:*?/\`.
fn sheet_name(base: &str, index: usize) -> String {
    let clean: String = base
        .chars()
        .filter(|c| !matches!(c, '[' | ']' | ':' | '*' | '?' | '/' | '\\'))
        .collect();
    let clean = if clean.trim().is_empty() {
        "Data".to_string()
    } else {
        clean
    };
    let name = if index == 0 {
        clean
    } else {
        format!("{clean} {}", index + 1)
    };
    name.chars().take(31).collect()
}

struct Formats {
    header: Format,
    time: Format,
    numbers: [Format; 5],
}

fn start_sheet<'a>(
    workbook: &'a mut Workbook,
    labels: &XlsxLabels,
    index: usize,
    f: &Formats,
) -> Result<&'a mut Worksheet> {
    let sheet = workbook
        .add_worksheet_with_constant_memory()
        .map_err(xlsx_err)?;
    sheet
        .set_name(sheet_name(&labels.sheet, index))
        .map_err(xlsx_err)?;
    let headers = [
        &labels.time,
        &labels.device,
        &labels.channel,
        &labels.temperature,
        &labels.humidity,
        &labels.illuminance,
        &labels.pressure,
        &labels.altitude,
    ];
    for (col, text) in headers.iter().enumerate() {
        sheet
            .write_string_with_format(0, col as u16, text.as_str(), &f.header)
            .map_err(xlsx_err)?;
    }
    for (col, width) in [20.0, 14.0, 12.0, 18.0, 16.0, 18.0, 16.0, 18.0]
        .iter()
        .enumerate()
    {
        sheet
            .set_column_width(col as u16, *width)
            .map_err(xlsx_err)?;
    }
    sheet.set_freeze_panes(1, 0).map_err(xlsx_err)?;
    Ok(sheet)
}

/// Export readings in `[from, to)` to `path`, one row per cycle and I2C
/// channel (written to `<path>.part`, renamed on success). `progress(done, total)` may return `false` to cancel.
pub fn export_xlsx_to_file(
    conn: &Connection,
    path: &Path,
    from: i64,
    to: i64,
    opts: CsvOptions,
    labels: &XlsxLabels,
    mut progress: impl FnMut(u64, u64) -> bool,
) -> Result<u64> {
    let total = super::count(conn, from, to)?;
    let formats = Formats {
        header: Format::new()
            .set_bold()
            .set_border_bottom(FormatBorder::Thin),
        time: Format::new().set_num_format(&labels.datetime_format),
        numbers: [
            Format::new().set_num_format("0.00"),
            Format::new().set_num_format("0.0"),
            Format::new().set_num_format("0.0"),
            Format::new().set_num_format("0.00"),
            Format::new().set_num_format("0.0"),
        ],
    };

    let mut workbook = Workbook::new();
    let mut stmt = conn.prepare(
        "SELECT r.ts, d.serial, r.temperature, r.humidity, r.illuminance, r.pressure, r.channel
         FROM readings r JOIN devices d ON d.id = r.device_id
         WHERE r.ts >= ?1 AND r.ts < ?2 ORDER BY r.ts, r.channel",
    )?;
    let mut rows = stmt.query(params![from, to])?;

    let mut n = 0u64;
    let mut sheet_index = 0usize;
    let mut row: u32 = 0;
    let mut sheet = start_sheet(&mut workbook, labels, sheet_index, &formats)?;
    let mut last_row_per_sheet = Vec::new();

    while let Some(r) = rows.next()? {
        if row == MAX_DATA_ROWS {
            last_row_per_sheet.push(row);
            sheet_index += 1;
            row = 0;
            sheet = start_sheet(&mut workbook, labels, sheet_index, &formats)?;
        }
        row += 1;
        let ts: i64 = r.get(0)?;
        let serial: String = r.get(1)?;
        let t: Option<f64> = r.get(2)?;
        let t = if opts.fahrenheit {
            t.map(celsius_to_fahrenheit)
        } else {
            t
        };
        let p: Option<f64> = r.get(5)?;
        let values = [
            t,
            r.get::<_, Option<f64>>(3)?,
            r.get::<_, Option<f64>>(4)?,
            p.map(|p| p / 100.0),
            p.map(|p| altitude(p, opts.sea_level_pa)),
        ];

        if let Some(local) = Local.timestamp_millis_opt(ts).single() {
            let seconds =
                f64::from(local.second()) + f64::from(local.timestamp_subsec_millis()) / 1000.0;
            let dt = ExcelDateTime::from_ymd(
                local.year() as u16,
                local.month() as u8,
                local.day() as u8,
            )
            .and_then(|d| d.and_hms(local.hour() as u16, local.minute() as u8, seconds))
            .map_err(xlsx_err)?;
            sheet
                .write_datetime_with_format(row, 0, &dt, &formats.time)
                .map_err(xlsx_err)?;
        }
        sheet.write_string(row, 1, serial).map_err(xlsx_err)?;
        let channel: i64 = r.get(6)?;
        sheet
            .write_number(row, 2, channel as f64)
            .map_err(xlsx_err)?;
        for (i, v) in values.iter().enumerate() {
            if let Some(v) = v {
                sheet
                    .write_number_with_format(row, 3 + i as u16, *v, &formats.numbers[i])
                    .map_err(xlsx_err)?;
            }
        }

        n += 1;
        if n.is_multiple_of(10_000) && !progress(n, total) {
            return Err(StoreError::Cancelled);
        }
    }
    last_row_per_sheet.push(row);
    for (i, last) in last_row_per_sheet.iter().enumerate() {
        if let Ok(s) = workbook.worksheet_from_index(i) {
            let _ = s.autofilter(0, 0, (*last).max(1), 7);
        }
    }

    let mut part = path.as_os_str().to_owned();
    part.push(".part");
    let part = std::path::PathBuf::from(part);
    match workbook.save(&part) {
        Ok(()) => {
            std::fs::rename(&part, path)?;
            progress(n, total);
            Ok(n)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(xlsx_err(e))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    use crate::store::tests::reading;

    fn labels() -> XlsxLabels {
        XlsxLabels {
            datetime_format: "d. m. yyyy hh:mm:ss".into(),
            sheet: "Měření".into(),
            time: "Čas".into(),
            device: "Zařízení".into(),
            channel: "Kanál I²C".into(),
            temperature: "Teplota (°C)".into(),
            humidity: "Vlhkost (%)".into(),
            illuminance: "Osvětlení (lx)".into(),
            pressure: "Tlak (hPa)".into(),
            altitude: "Nadmořská výška (m)".into(),
        }
    }

    #[test]
    fn writes_a_valid_workbook() -> Result<()> {
        let mut s = Store::open_in_memory()?;
        for i in 0..100 {
            s.insert(&reading(
                1_700_000_000_000 + i * 10_000,
                20.0 + i as f64 / 10.0,
            ))?;
        }
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("out.xlsx");
        let opts = CsvOptions {
            fahrenheit: false,
            sea_level_pa: 101_325.0,
        };
        let n = export_xlsx_to_file(s.conn(), &path, 0, i64::MAX, opts, &labels(), |_, _| true)?;
        assert_eq!(n, 100);
        let bytes = std::fs::read(&path)?;
        assert_eq!(&bytes[..2], b"PK", "xlsx is a zip archive");
        assert!(!dir.path().join("out.xlsx.part").exists());
        Ok(())
    }

    #[test]
    fn sheet_names_are_valid() {
        assert_eq!(sheet_name("Měření", 0), "Měření");
        assert_eq!(sheet_name("Měření", 1), "Měření 2");
        assert_eq!(sheet_name("a/b:c*?", 0), "abc");
        assert_eq!(sheet_name(&"x".repeat(40), 0).chars().count(), 31);
    }
}
