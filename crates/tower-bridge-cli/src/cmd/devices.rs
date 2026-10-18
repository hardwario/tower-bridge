//! `relay`, `gps` and `nfc`.

use std::time::Duration;

use anyhow::Result;
use tower_bridge_core::model::DeviceState;
use tower_bridge_core::nfc::{MAX_URL, NfcInfo, with_scheme};
use tower_bridge_core::relay::RelayInfo;

use super::status::{gps_summary, relay_text, session};
use super::{Out, line};
use crate::access::{Access, Ctx, device_failure};
use crate::cli::{NfcAction, RelayAction, RelayState};
use crate::{exit, output};

fn print_relay(out: Out, r: &RelayInfo) -> Result<()> {
    if out.json {
        return output::json(r);
    }
    line(out, "TOWER Relay Module", &relay_text(out, r.state));
    line(
        out,
        "  Place",
        &format!("I²C channel {}, address 0x{:02X}", r.channel, r.address),
    );
    if let Some(until) = r.pulse_until {
        line(
            out,
            "  Pulse",
            &format!(
                "until {}, then {}",
                output::time(until),
                output::relay_state(r.state.map(|s| !s))
            ),
        );
    }
    Ok(())
}

pub fn relay(ctx: &Ctx, out: Out, action: Option<RelayAction>) -> Result<()> {
    let (access, snap) = session(ctx)?;
    let result = (|| {
        if let Some(f) = device_failure(&snap.device) {
            return Err(f);
        }
        if snap.relay.is_none() {
            return Err(crate::failure(
                exit::NOT_FOUND,
                "no TOWER Relay Module is connected",
                None,
            ));
        }
        let info = match action {
            None => snap.relay,
            Some(RelayAction::True) => Some(access.set_relay(true)?),
            Some(RelayAction::False) => Some(access.set_relay(false)?),
            Some(RelayAction::Pulse { state, ms }) => {
                let info = access.pulse_relay(state.map(|s| s == RelayState::True), ms)?;
                print_relay(out, &info)?;
                // The service here switches the relay back: it must run
                // until the pulse ends. Ctrl+C ends the pulse now, so the
                // relay is never left switched over.
                if let (Access::Local(_), Some(until)) = (&access, info.pulse_until) {
                    let left = until - tower_bridge_core::acquisition::unix_ms();
                    if left > 0 {
                        eprintln!("Waiting for the pulse to end; Ctrl+C ends it now.");
                        let stop = super::watch::stop_signal();
                        let wait = Duration::from_millis(left.unsigned_abs() + 400);
                        if stop.recv_timeout(wait).is_ok()
                            && let Some(back) = info.state.map(|s| !s)
                        {
                            print_relay(out, &access.set_relay(back)?)?;
                        }
                    }
                }
                return Ok(());
            }
        };
        match info {
            Some(info) => print_relay(out, &info),
            None => Ok(()),
        }
    })();
    access.close();
    result
}

pub fn gps(ctx: &Ctx, out: Out) -> Result<()> {
    let (access, mut snap) = session(ctx)?;
    // A receiver is announced with its first report, a few seconds after the
    // module was found.
    let found = matches!(
        snap.device,
        DeviceState::Ready { .. } | DeviceState::NoModule { .. }
    );
    if matches!(access, Access::Local(_)) && found {
        let until = std::time::Instant::now() + Duration::from_secs(12);
        while snap.gps.is_none() && std::time::Instant::now() < until {
            std::thread::sleep(Duration::from_millis(200));
            snap = access.snapshot()?;
        }
    }
    access.close();
    if let Some(f) = device_failure(&snap.device) {
        return Err(f);
    }
    let Some(g) = snap.gps else {
        return Err(crate::failure(
            exit::NOT_FOUND,
            "no TOWER GPS Module is connected",
            None,
        ));
    };
    if out.json {
        return output::json(&g);
    }
    line(out, "TOWER GPS Module", &gps_summary(&g));
    line(out, "  Place", &format!("I²C channel {}", g.channel));
    if let Some(t) = g.time {
        line(out, "  Satellite time", &output::time(t));
    }
    Ok(())
}

fn print_nfc(out: Out, n: &NfcInfo) -> Result<()> {
    if out.json {
        return output::json(n);
    }
    let url = match (&n.url, n.formatted) {
        (Some(url), _) => url.clone(),
        (None, true) => out.style.dim("no URL"),
        (None, false) => out.style.warn("not NDEF formatted"),
    };
    line(out, "TOWER NFC Tag", &url);
    line(
        out,
        "  Place",
        &format!(
            "I²C channel {}, address 0x{:02X} ({})",
            n.channel, n.address, n.part
        ),
    );
    Ok(())
}

pub fn nfc(ctx: &Ctx, out: Out, action: Option<NfcAction>) -> Result<()> {
    // Checked before the bridge is opened.
    let url = match &action {
        Some(NfcAction::Write { url }) => {
            let url = with_scheme(url);
            if url.is_empty() || url.len() > MAX_URL {
                return Err(crate::failure(
                    exit::USAGE,
                    format!(
                        "the URL must have 1 to {MAX_URL} bytes, this one has {}",
                        url.len()
                    ),
                    None,
                ));
            }
            Some(url)
        }
        None => None,
    };
    let (access, snap) = session(ctx)?;
    let result = (|| {
        if let Some(f) = device_failure(&snap.device) {
            return Err(f);
        }
        let Some(tag) = &snap.nfc else {
            return Err(crate::failure(
                exit::NOT_FOUND,
                "no TOWER NFC Tag is connected",
                None,
            ));
        };
        if url.is_some() && !tag.formatted {
            return Err(crate::failure(
                exit::ERROR,
                "the TOWER NFC Tag is not NDEF formatted",
                Some("format it with a phone app (for example NXP TagWriter), then write the URL"),
            ));
        }
        match &url {
            Some(url) => print_nfc(out, &access.write_nfc(url)?),
            None => print_nfc(out, tag),
        }
    })();
    access.close();
    if result.is_ok() && url.is_some() && !out.json {
        eprintln!("Written; phones open it when they touch the tag.");
    }
    result
}
