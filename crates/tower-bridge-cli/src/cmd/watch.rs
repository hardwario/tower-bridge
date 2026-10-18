//! `watch`: readings as they come, as text lines or NDJSON, with the
//! changes of the bridge, the relay, the NFC Tag, the GPS position and the
//! alerts.

use std::collections::HashMap;
use std::io::Write;
use std::time::Duration;

use anyhow::Result;
use crossbeam_channel::{Receiver, select, tick};
use serde_json::{Value, json};
use tower_bridge_core::alerts::{ActiveAlert, AlertEvent, AlertKind};
use tower_bridge_core::gps::GpsInfo;
use tower_bridge_core::model::{DeviceState, Metric, Reading};
use tower_bridge_core::nfc::NfcInfo;
use tower_bridge_core::relay::RelayInfo;
use tower_bridge_service::Mode;
use tower_bridge_service::settings::TemperatureUnit;

use super::Out;
use crate::access::{Access, Ctx, Event, Snapshot, device_failure, events};
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
    let mut printer = Printer {
        out,
        unit,
        count,
        printed: 0,
        gps_fix: None,
    };
    let result = match &access {
        Access::Local(_) => follow(&mut printer, &rx, &stop),
        Access::Remote(_) => {
            eprintln!("Following {}.", access.describe());
            poll(&access, &mut printer, &stop)
        }
    };
    access.close();
    result
}

/// Prints what `watch` reports.
struct Printer {
    out: Out,
    unit: TemperatureUnit,
    count: Option<u32>,
    printed: u32,
    /// The last GPS fix printed as text (the position comes every 5 s).
    gps_fix: Option<Option<bool>>,
}

impl Printer {
    /// One reading; `true` once `--count` readings are out.
    fn reading(&mut self, r: &Reading) -> Result<bool> {
        let mut stdout = std::io::stdout().lock();
        if self.out.json {
            let mut v = serde_json::to_value(r)?;
            if let Value::Object(map) = &mut v {
                map.insert("type".into(), json!("reading"));
            }
            writeln!(stdout, "{v}")?;
        } else {
            let mut text = format!(
                "{}  {}",
                output::time(r.ts),
                self.out.style.dim(&format!("I²C {}", r.channel))
            );
            for metric in Metric::ALL {
                if let Some(v) = r.values.get(metric) {
                    text += &format!(
                        "  {} {}",
                        output::metric_name(metric),
                        output::value(metric, Some(v), self.unit)
                    );
                }
            }
            writeln!(stdout, "{text}")?;
        }
        stdout.flush()?;
        self.printed += 1;
        Ok(self.count.is_some_and(|n| self.printed >= n))
    }

    /// A change of state: a JSON line on stdout, or a message on stderr.
    fn meta(&self, kind: &str, mut value: Value, text: &str) -> Result<()> {
        if self.out.json {
            if let Value::Object(map) = &mut value {
                map.insert("type".into(), json!(kind));
            }
            let mut stdout = std::io::stdout().lock();
            writeln!(stdout, "{value}")?;
            stdout.flush()?;
        } else if !text.is_empty() {
            eprintln!("{text}");
        }
        Ok(())
    }

    fn device(&self, d: &DeviceState) -> Result<()> {
        self.meta(
            "device",
            json!({ "device": d }),
            &format!("bridge: {}", output::device(d)),
        )
    }

    fn relay(&self, r: Option<&RelayInfo>) -> Result<()> {
        self.meta(
            "relay",
            json!({ "relay": r }),
            &format!(
                "relay: {}",
                r.map_or("not connected", |r| output::relay_state(r.state))
            ),
        )
    }

    fn nfc(&self, n: Option<&NfcInfo>) -> Result<()> {
        self.meta(
            "nfc",
            json!({ "nfc": n }),
            &format!(
                "NFC Tag: {}",
                n.map_or("not connected", |n| n.url.as_deref().unwrap_or("no URL"))
            ),
        )
    }

    fn gps(&mut self, g: Option<&GpsInfo>) -> Result<()> {
        if self.out.json {
            return self.meta("gps", json!({ "gps": g }), "");
        }
        // As text, only a change of fix is news.
        let fix = g.map(|g| g.fix);
        if self.gps_fix != Some(fix) {
            self.gps_fix = Some(fix);
            eprintln!(
                "GPS: {}",
                g.map_or("not connected".into(), super::status::gps_summary)
            );
        }
        Ok(())
    }

    fn alert(&self, a: &AlertEvent) -> Result<()> {
        let text = output::alert(a, self.unit);
        let text = match a.kind {
            AlertKind::Triggered => format!("alert: {text}"),
            AlertKind::Recovered => format!("alert over: {text}"),
        };
        self.meta("alert", json!({ "alert": a }), &text)
    }
}

/// Print the reports of the service running here until stopped.
fn follow(p: &mut Printer, rx: &Receiver<Event>, stop: &Receiver<()>) -> Result<()> {
    // Searching comes first; the next state says whether there is a bridge.
    let mut found = false;
    loop {
        select! {
            recv(stop) -> _ => return Ok(()),
            recv(rx) -> ev => {
                let Ok(ev) = ev else { return Ok(()) };
                match ev {
                    Event::Reading(r) => {
                        if p.reading(&r)? {
                            return Ok(());
                        }
                    }
                    Event::Device(d) => {
                        if !found && !matches!(d, DeviceState::Searching) {
                            found = true;
                            if let Some(f) = device_failure(&d) {
                                return Err(f);
                            }
                        }
                        p.device(&d)?;
                    }
                    Event::Relay(r) => p.relay(r.as_ref())?,
                    Event::Nfc(n) => p.nfc(n.as_ref())?,
                    Event::Gps(g) => p.gps(g.as_ref())?,
                    Event::Alert(a) => p.alert(&a)?,
                    Event::Mqtt(_) | Event::Refresh => {}
                }
            }
        }
    }
}

/// An alert event from a change of the active alerts.
fn alert_event(a: &ActiveAlert, kind: AlertKind, ts: i64, value: f64) -> AlertEvent {
    AlertEvent {
        ts,
        metric: a.metric,
        channel: a.channel,
        bound: a.bound,
        kind,
        value,
        threshold: a.threshold,
        notify: true,
    }
}

/// Follow a running instance by asking for its state every second; report
/// what changed.
fn poll(access: &Access, p: &mut Printer, stop: &Receiver<()>) -> Result<()> {
    let ticker = tick(Duration::from_secs(1));
    let mut seen: HashMap<u8, i64> = HashMap::new();
    // What is there already is not new.
    let mut last: Snapshot = access.snapshot()?;
    for r in &last.latest {
        seen.insert(r.channel, r.ts);
    }
    loop {
        select! {
            recv(stop) -> _ => return Ok(()),
            recv(ticker) -> _ => {
                let now = access.snapshot()?;
                if now.device != last.device {
                    p.device(&now.device)?;
                }
                if now.relay != last.relay {
                    p.relay(now.relay.as_ref())?;
                }
                if now.nfc != last.nfc {
                    p.nfc(now.nfc.as_ref())?;
                }
                if now.gps != last.gps {
                    p.gps(now.gps.as_ref())?;
                }
                let key = |a: &ActiveAlert| (a.metric, a.channel, a.bound);
                for a in &now.active_alerts {
                    if !last.active_alerts.iter().any(|b| key(b) == key(a)) {
                        p.alert(&alert_event(a, AlertKind::Triggered, a.since, a.value))?;
                    }
                }
                for a in &last.active_alerts {
                    if !now.active_alerts.iter().any(|b| key(b) == key(a)) {
                        let value = now
                            .latest
                            .iter()
                            .find(|r| r.channel == a.channel)
                            .and_then(|r| r.values.get(a.metric))
                            .unwrap_or(a.value);
                        let ts = tower_bridge_core::acquisition::unix_ms();
                        p.alert(&alert_event(a, AlertKind::Recovered, ts, value))?;
                    }
                }
                for r in &now.latest {
                    if seen.get(&r.channel).is_some_and(|ts| *ts >= r.ts) {
                        continue;
                    }
                    seen.insert(r.channel, r.ts);
                    if p.reading(r)? {
                        return Ok(());
                    }
                }
                last = now;
            }
        }
    }
}
