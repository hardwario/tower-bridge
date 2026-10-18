//! `watch`: readings as they come, as text lines or NDJSON.

use std::collections::HashMap;
use std::io::Write;
use std::time::Duration;

use anyhow::Result;
use crossbeam_channel::{Receiver, select, tick};
use serde_json::{Value, json};
use tower_bridge_core::model::{DeviceState, Metric, Reading};
use tower_bridge_service::Mode;
use tower_bridge_service::settings::TemperatureUnit;

use super::Out;
use crate::access::{Access, Ctx, Event, device_failure, events};
use crate::output;

/// A channel that fires once on Ctrl+C, SIGTERM or SIGHUP.
pub fn stop_signal() -> Receiver<()> {
    let (tx, rx) = crossbeam_channel::bounded(1);
    if let Err(e) = ctrlc::set_handler(move || {
        let _ = tx.try_send(());
    }) {
        tracing::warn!("cannot catch Ctrl+C: {e}");
    }
    rx
}

pub fn watch(ctx: &Ctx, out: Out, count: Option<u32>) -> Result<()> {
    let stop = stop_signal();
    let (observer, rx) = events();
    let access = Access::open(ctx, Mode::Session, observer)?;
    let unit = ctx.settings()?.temperature_unit;
    let result = match &access {
        Access::Local(_) => follow(out, unit, &rx, &stop, count),
        Access::Remote(_) => {
            eprintln!("Following {}.", access.describe());
            poll(&access, out, unit, &stop, count)
        }
    };
    access.close();
    result
}

/// Print the service's reports until stopped.
fn follow(
    out: Out,
    unit: TemperatureUnit,
    rx: &Receiver<Event>,
    stop: &Receiver<()>,
    count: Option<u32>,
) -> Result<()> {
    let mut printed = 0u32;
    let mut device: Option<DeviceState> = None;
    let mut gps_fix: Option<bool> = None;
    loop {
        select! {
            recv(stop) -> _ => return Ok(()),
            recv(rx) -> ev => {
                let Ok(ev) = ev else { return Ok(()) };
                match ev {
                    Event::Reading(r) => {
                        reading(out, unit, &r)?;
                        printed += 1;
                        if count.is_some_and(|n| printed >= n) {
                            return Ok(());
                        }
                    }
                    Event::Device(d) => {
                        // A bridge that cannot be used, once it has been searched for.
                        if device.is_none()
                            && let Some(f) = device_failure(&d)
                            && !matches!(d, DeviceState::Searching)
                        {
                            return Err(f);
                        }
                        meta(out, "device", json!({ "device": &d }), &format!("bridge: {}", output::device(&d)))?;
                        device = Some(d);
                    }
                    Event::Relay(r) => meta(
                        out,
                        "relay",
                        json!({ "relay": r }),
                        &format!("relay: {}", r.map_or("not connected", |r| output::relay_state(r.state))),
                    )?,
                    Event::Alert(a) => meta(out, "alert", json!({ "alert": &a }), &format!("alert: {}", output::alert(&a, unit)))?,
                    Event::Nfc(n) => meta(
                        out,
                        "nfc",
                        json!({ "nfc": &n }),
                        &format!("NFC Tag: {}", n.as_ref().map_or("not connected", |n| n.url.as_deref().unwrap_or("no URL"))),
                    )?,
                    Event::Gps(g) => {
                        // Every 5 s; only a change of fix is news in text.
                        if out.json {
                            meta(out, "gps", json!({ "gps": &g }), "")?;
                        } else if g.as_ref().map(|g| g.fix) != gps_fix {
                            gps_fix = g.as_ref().map(|g| g.fix);
                            eprintln!("GPS: {}", g.as_ref().map_or("not connected".into(), super::status::gps_summary));
                        }
                    }
                    Event::Mqtt(_) | Event::Refresh => {}
                }
            }
        }
    }
}

/// Follow a running instance by asking for its latest readings.
fn poll(
    access: &Access,
    out: Out,
    unit: TemperatureUnit,
    stop: &Receiver<()>,
    count: Option<u32>,
) -> Result<()> {
    let ticker = tick(Duration::from_secs(1));
    let mut seen: HashMap<u8, i64> = HashMap::new();
    let mut printed = 0u32;
    // What is there already is not new.
    for r in access.snapshot()?.latest {
        seen.insert(r.channel, r.ts);
    }
    loop {
        select! {
            recv(stop) -> _ => return Ok(()),
            recv(ticker) -> _ => {
                for r in access.snapshot()?.latest {
                    if seen.get(&r.channel).is_some_and(|ts| *ts >= r.ts) {
                        continue;
                    }
                    seen.insert(r.channel, r.ts);
                    reading(out, unit, &r)?;
                    printed += 1;
                    if count.is_some_and(|n| printed >= n) {
                        return Ok(());
                    }
                }
            }
        }
    }
}

/// One reading: an NDJSON line, else "10:05:22  I²C 0  Temperature 23.4 °C  ...".
fn reading(out: Out, unit: TemperatureUnit, r: &Reading) -> Result<()> {
    let mut stdout = std::io::stdout().lock();
    if out.json {
        let mut v = serde_json::to_value(r)?;
        if let Value::Object(map) = &mut v {
            map.insert("type".into(), json!("reading"));
        }
        writeln!(stdout, "{v}")?;
    } else {
        let mut text = format!(
            "{}  {}",
            output::time(r.ts),
            out.style.dim(&format!("I²C {}", r.channel))
        );
        for metric in Metric::ALL {
            if let Some(v) = r.values.get(metric) {
                text += &format!(
                    "  {} {}",
                    output::metric_name(metric),
                    output::value(metric, Some(v), unit)
                );
            }
        }
        writeln!(stdout, "{text}")?;
    }
    stdout.flush()?;
    Ok(())
}

/// A change of state: a JSON line on stdout, or a message on stderr.
fn meta(out: Out, kind: &str, mut value: Value, text: &str) -> Result<()> {
    if out.json {
        if let Value::Object(map) = &mut value {
            map.insert("type".into(), json!(kind));
        }
        let mut stdout = std::io::stdout().lock();
        writeln!(stdout, "{value}")?;
        stdout.flush()?;
    } else {
        eprintln!("{text}");
    }
    Ok(())
}
