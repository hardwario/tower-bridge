//! `export` and `alerts`: the history in the database the app and the CLI
//! share. Reading it needs no bridge, also while the app runs.

use std::path::Path;

use anyhow::{Context, Result};
use serde_json::json;
use tower_bridge_core::acquisition::unix_ms;
use tower_bridge_core::alerts::AlertRule;
use tower_bridge_core::model::Metric;
use tower_bridge_core::store::{self, CsvOptions, XlsxLabels};
use tower_bridge_service::lock::InstanceLock;
use tower_bridge_service::settings::{Settings, TemperatureUnit};

use super::{Out, line};
use crate::access::Ctx;
use crate::cli::ExportArgs;
use crate::remote::Remote;
use crate::{exit, output};

fn reader(ctx: &Ctx) -> Result<store::Connection> {
    let db = ctx.paths()?.db_file();
    if !db.exists() {
        return Err(crate::failure(
            exit::NOT_FOUND,
            format!("no history yet: {} does not exist", db.display()),
            Some("`tower-bridge-cli run` or the desktop app records it"),
        ));
    }
    store::open_reader(&db).with_context(|| format!("opening {}", db.display()))
}

/// The range of an export: `--last`, or `--from` and `--to`.
fn range(args: &ExportArgs) -> Result<(i64, i64)> {
    let now = unix_ms();
    let bad = |what: &str, value: &str| {
        crate::failure(
            exit::USAGE,
            format!("cannot read {what} {value:?}"),
            Some(
                "use a date (2026-10-17), a local time (2026-10-17T08:00), RFC 3339 or Unix ms; spans like 30m, 24h, 7d",
            ),
        )
    };
    if args.from.is_none() && args.to.is_none() {
        let last = args.last.as_deref().unwrap_or("24h");
        let span = output::parse_span(last).ok_or_else(|| bad("the span", last))?;
        return Ok((now - span, now));
    }
    let to = match &args.to {
        Some(t) => output::parse_time(t).ok_or_else(|| bad("--to", t))?,
        None => now,
    };
    let from = match &args.from {
        Some(f) => output::parse_time(f).ok_or_else(|| bad("--from", f))?,
        None => to - 86_400_000,
    };
    if from >= to {
        return Err(crate::failure(
            exit::USAGE,
            "--from must be before --to",
            None,
        ));
    }
    Ok((from, to))
}

fn labels(unit: TemperatureUnit) -> XlsxLabels {
    let temperature = if unit == TemperatureUnit::Fahrenheit {
        "°F"
    } else {
        "°C"
    };
    XlsxLabels {
        datetime_format: "yyyy-mm-dd hh:mm:ss".into(),
        sheet: "Readings".into(),
        time: "Time".into(),
        device: "Device".into(),
        channel: "I2C channel".into(),
        temperature: format!("Temperature ({temperature})"),
        humidity: "Humidity (%)".into(),
        illuminance: "Illuminance (lx)".into(),
        pressure: "Pressure (hPa)".into(),
        altitude: "Altitude (m)".into(),
        co2: "CO2 (ppm)".into(),
        voc: "VOC (ppb)".into(),
    }
}

pub fn export(ctx: &Ctx, args: &ExportArgs) -> Result<()> {
    let (from, to) = range(args)?;
    let conn = reader(ctx)?;
    let settings = ctx.settings()?;
    let opts = CsvOptions {
        fahrenheit: settings.temperature_unit == TemperatureUnit::Fahrenheit,
        sea_level_pa: settings.sea_level_pressure_pa,
    };
    let rows = match &args.output {
        None => store::write_csv(&conn, from, to, opts, std::io::stdout().lock(), |_, _| true)?,
        Some(path) if is_xlsx(path) => store::export_xlsx_to_file(
            &conn,
            path,
            from,
            to,
            opts,
            &labels(settings.temperature_unit),
            |_, _| true,
        )?,
        Some(path) => store::export_to_file(&conn, path, from, to, opts, |_, _| true)?,
    };
    let to_where = args
        .output
        .as_ref()
        .map_or("stdout".to_string(), |p| p.display().to_string());
    eprintln!(
        "{rows} readings from {} to {} written to {to_where}.",
        output::time(from),
        output::time(to)
    );
    Ok(())
}

fn is_xlsx(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("xlsx"))
}

fn rule_text(metric: Metric, rule: &AlertRule, unit: TemperatureUnit) -> String {
    if !rule.enabled {
        return "off".into();
    }
    let mut parts = Vec::new();
    if let Some(min) = rule.min {
        parts.push(format!("below {}", output::value(metric, Some(min), unit)));
    }
    if let Some(max) = rule.max {
        parts.push(format!("above {}", output::value(metric, Some(max), unit)));
    }
    if parts.is_empty() {
        parts.push("no limits".into());
    }
    // A difference: no offset in °F, only the scale.
    let hysteresis = match (metric, unit) {
        (Metric::Temperature, TemperatureUnit::Fahrenheit) => {
            format!("{:.1} °F", rule.hysteresis * 9.0 / 5.0)
        }
        (Metric::Pressure, _) => format!("{:.1} hPa", rule.hysteresis / 100.0),
        _ => format!("{} {}", rule.hysteresis, output::unit_label(metric, unit)),
    };
    format!(
        "{}, hysteresis {hysteresis}, repeat after {} min",
        parts.join(", "),
        rule.cooldown_ms / 60_000
    )
}

pub fn alerts(ctx: &Ctx, out: Out, log_len: usize) -> Result<()> {
    let paths = ctx.paths()?;
    let settings = Settings::read(&paths.settings_file());
    let unit = settings.temperature_unit;
    // Active alerts live in the running service.
    let active = match InstanceLock::holder(&paths.lock_file()) {
        Some(holder) if settings.api.enabled => Some(
            Remote::new(&settings, holder, paths.settings_file())
                .status()?
                .active_alerts,
        ),
        _ => None,
    };
    let log = match reader(ctx) {
        Ok(conn) => store::alert_log(&conn, log_len)?,
        Err(_) => Vec::new(),
    };
    if out.json {
        return output::json(&json!({ "rules": settings.alerts, "active": active, "log": log }));
    }
    let s = out.style;
    println!("{}", s.bold("Rules"));
    for metric in Metric::ALL {
        line(
            out,
            &format!("  {}", output::metric_name(metric)),
            &rule_text(metric, settings.alerts.get(metric), unit),
        );
    }
    println!("{}", s.bold("Active"));
    match &active {
        None => println!(
            "  {}",
            s.dim("not known: no service with an API server runs")
        ),
        Some(a) if a.is_empty() => println!("  {}", s.dim("none")),
        Some(list) => {
            for a in list {
                let bound = match a.bound {
                    tower_bridge_core::alerts::Bound::High => "above",
                    tower_bridge_core::alerts::Bound::Low => "below",
                };
                println!(
                    "  {}",
                    s.bad(&format!(
                        "{} on I²C channel {} {bound} {}: {}, since {}",
                        output::metric_name(a.metric),
                        a.channel,
                        output::value(a.metric, Some(a.threshold), unit),
                        output::value(a.metric, Some(a.value), unit),
                        output::time(a.since)
                    ))
                );
            }
        }
    }
    println!("{}", s.bold("Recent"));
    if log.is_empty() {
        println!("  {}", s.dim("none"));
    }
    for e in &log {
        let Ok(metric) = serde_json::from_value::<Metric>(json!(e.metric)) else {
            continue;
        };
        let what = match (e.kind.as_str(), e.bound.as_str()) {
            ("recovered", _) => s.good("back to normal"),
            (_, "high") => s.bad("above the limit"),
            _ => s.bad("below the limit"),
        };
        println!(
            "  {}  {} on I²C channel {} {what}: {} (limit {})",
            output::time(e.ts),
            output::metric_name(metric),
            e.channel,
            output::value(metric, Some(e.value), unit),
            output::value(metric, Some(e.threshold), unit),
        );
    }
    Ok(())
}
