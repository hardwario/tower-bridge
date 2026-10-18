//! `status`, `identify` and `rescan`.

use std::io::IsTerminal;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use serde_json::json;
use tower_bridge_core::model::{DeviceState, Metric};
use tower_bridge_service::{Mode, MqttState, Silent};

use super::{Out, line};
use crate::access::{Access, Ctx, Snapshot, device_failure};
use crate::output;

/// How long a short session waits for the bridge and the first values.
pub const SETTLE: Duration = Duration::from_secs(15);

/// Open the bridge (or reach the running service) and wait for the values.
pub fn session(ctx: &Ctx) -> Result<(Access, Snapshot)> {
    let access = Access::open(ctx, Mode::Session, Arc::new(Silent))?;
    // The first measurement takes a few seconds; say so in a terminal.
    let note = matches!(access, Access::Local(_)) && std::io::stderr().is_terminal();
    if note {
        eprint!("Measuring…");
    }
    let snap = access.settle(SETTLE);
    if note {
        eprint!("\r\x1b[K");
    }
    Ok((access, snap?))
}

pub fn status(ctx: &Ctx, out: Out) -> Result<()> {
    let (access, snap) = session(ctx)?;
    let failure = device_failure(&snap.device);
    if out.json {
        output::json(&json!({ "via": access.describe(), "status": &snap }))?;
    } else if failure.is_none() {
        print_snapshot(out, &snap, &access.describe());
    }
    access.close();
    failure.map_or(Ok(()), Err)
}

/// The bridge, the modules per channel with their values, and the rest.
pub fn print_snapshot(out: Out, snap: &Snapshot, via: &str) {
    let s = out.style;
    let state = output::device(&snap.device);
    let state = if snap.device.is_ready() {
        s.good(&state)
    } else {
        s.warn(&state)
    };
    line(out, "TOWER Bridge Module", &state);
    let sources = snap.device.sources();
    let mut channels: Vec<u8> = sources.iter().map(|x| x.channel).collect();
    channels.sort_unstable();
    channels.dedup();
    for channel in channels {
        line(
            out,
            &format!("I²C channel {channel}"),
            &output::modules_on(sources, channel).join(", "),
        );
        let reading = snap.latest.iter().find(|r| r.channel == channel);
        for metric in Metric::ALL {
            if !sources
                .iter()
                .any(|x| x.channel == channel && x.metric == metric)
            {
                continue;
            }
            let v = reading.and_then(|r| r.values.get(metric));
            let text = match v {
                Some(_) => output::value(metric, v, snap.unit),
                None => s.dim("waiting for a value"),
            };
            line(out, &format!("  {}", output::metric_name(metric)), &text);
            if metric == Metric::Pressure
                && let Some(alt) = reading.and_then(|r| r.altitude)
            {
                line(out, "  Altitude", &format!("{alt:.0} m"));
            }
        }
    }
    if let Some(r) = snap.relay {
        line(
            out,
            "Relay",
            &format!(
                "{}  {}",
                relay_text(out, r.state),
                s.dim(&format!("I²C channel {}, 0x{:02X}", r.channel, r.address))
            ),
        );
    }
    if let Some(g) = &snap.gps {
        line(out, "GPS", &gps_summary(g));
    }
    if let Some(n) = &snap.nfc {
        line(
            out,
            "NFC",
            &format!(
                "{}  {}",
                n.url.as_deref().unwrap_or("no URL"),
                s.dim(&format!("I²C channel {}, 0x{:02X}", n.channel, n.address))
            ),
        );
    }
    if let Some(m) = &snap.mqtt {
        line(out, "MQTT", &mqtt_text(out, m));
    }
    for a in &snap.active_alerts {
        let limit = output::value(a.metric, Some(a.threshold), snap.unit);
        let value = output::value(a.metric, Some(a.value), snap.unit);
        let bound = match a.bound {
            tower_bridge_core::alerts::Bound::High => "above",
            tower_bridge_core::alerts::Bound::Low => "below",
        };
        line(
            out,
            "Alert",
            &s.bad(&format!(
                "{} on I²C channel {} {bound} {limit}: {value}, since {}",
                output::metric_name(a.metric),
                a.channel,
                output::time(a.since)
            )),
        );
    }
    if let Some(ts) = snap.latest.iter().map(|r| r.ts).max() {
        line(
            out,
            "Updated",
            &format!("{}  {}", output::time(ts), s.dim(&format!("via {via}"))),
        );
    }
}

pub fn relay_text(out: Out, state: Option<bool>) -> String {
    let text = output::relay_state(state);
    match state {
        Some(true) => out.style.good(text),
        Some(false) => out.style.bad(text),
        None => out.style.dim(text),
    }
}

pub fn gps_summary(g: &tower_bridge_core::gps::GpsInfo) -> String {
    match (g.fix, g.latitude, g.longitude) {
        (true, Some(lat), Some(lon)) => {
            let mut s = format!("{lat:.6}, {lon:.6}");
            if let Some(acc) = g.accuracy {
                s += &format!(" (±{acc:.0} m)");
            }
            if let Some(alt) = g.altitude {
                s += &format!(", {alt:.0} m above sea level");
            }
            s + &format!(", {} satellites", g.satellites)
        }
        _ => format!("searching for satellites ({} in use)", g.satellites),
    }
}

pub fn mqtt_text(out: Out, m: &MqttState) -> String {
    match m {
        MqttState::Disabled => out.style.dim("off"),
        MqttState::Connecting { address, error } => out.style.warn(&match error {
            Some(e) => format!("connecting to {address} ({e})"),
            None => format!("connecting to {address}"),
        }),
        MqttState::Connected { address } => out.style.good(&format!("connected to {address}")),
        MqttState::Error { message } if message == "no_host" => {
            out.style.bad("no broker set (mqtt.host)")
        }
        MqttState::Error { message } => out.style.bad(message),
    }
}

pub fn identify(ctx: &Ctx, out: Out) -> Result<()> {
    let (access, snap) = session(ctx)?;
    if let Some(f) = device_failure(&snap.device) {
        access.close();
        return Err(f);
    }
    let result = access.identify();
    access.close();
    result?;
    if out.json {
        output::json(&json!({ "ok": true }))?;
    } else {
        println!("The LED on the TOWER Bridge Module blinked.");
    }
    Ok(())
}

pub fn rescan(ctx: &Ctx, out: Out) -> Result<()> {
    let (access, snap) = session(ctx)?;
    if let Some(f) = device_failure(&snap.device) {
        access.close();
        return Err(f);
    }
    let result = access.rescan().and_then(|()| access.snapshot());
    let via = access.describe();
    access.close();
    let snap = result?;
    if out.json {
        output::json(&json!({ "via": via, "status": &snap }))?;
    } else if matches!(snap.device, DeviceState::NoModule { .. }) {
        println!("No modules found.");
    } else {
        print_snapshot(out, &snap, &via);
    }
    Ok(())
}
