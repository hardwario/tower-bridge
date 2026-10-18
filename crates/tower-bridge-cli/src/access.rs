//! How a command reaches the bridge. Only one program can open it:
//!
//! - when the desktop app or `tower-bridge-cli run` holds the data folder
//!   (its instance lock), the command goes through its API server;
//! - otherwise the command starts the service itself: a short session for
//!   one-off commands (nothing recorded or published), or the whole service
//!   for `run` and the dashboard.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossbeam_channel::{Receiver, Sender};
use serde::Serialize;
use tower_bridge_core::acquisition::AcqError;
use tower_bridge_core::alerts::{ActiveAlert, AlertEvent};
use tower_bridge_core::gps::GpsInfo;
use tower_bridge_core::model::{DeviceErrorKind, DeviceState, Reading};
use tower_bridge_core::nfc::NfcInfo;
use tower_bridge_core::relay::RelayInfo;
use tower_bridge_core::store::{self, History, HistoryQuery};
use tower_bridge_service::lock::InstanceLock;
use tower_bridge_service::settings::{Settings, TemperatureUnit};
use tower_bridge_service::{
    Mode, MqttState, Observer, Paths, Service, ServiceConfig, ServiceState, StartError,
};

use crate::exit;
use crate::remote::Remote;

/// The global options every command needs.
#[derive(Clone, Debug)]
pub struct Ctx {
    pub data_dir: Option<PathBuf>,
    pub simulate: bool,
}

impl Ctx {
    pub fn paths(&self) -> Result<Paths> {
        Paths::resolve(self.data_dir.clone(), self.simulate).ok_or_else(|| {
            anyhow::anyhow!("cannot determine the data folder; give one with --data-dir")
        })
    }

    /// The settings as stored (defaults when there is no file yet).
    pub fn settings(&self) -> Result<Settings> {
        Ok(Settings::read(&self.paths()?.settings_file()))
    }
}

/// What the service reports, for `watch` and the dashboard.
#[derive(Clone, Debug)]
pub enum Event {
    Reading(Reading),
    Device(DeviceState),
    Relay(Option<RelayInfo>),
    Nfc(Option<NfcInfo>),
    Gps(Option<GpsInfo>),
    Alert(AlertEvent),
    Mqtt(MqttState),
    /// Something shown may have changed (also every few seconds).
    Refresh,
}

/// Forwards the service's reports to a channel.
pub struct Forward(pub Sender<Event>);

impl Observer for Forward {
    fn refresh(&self, _: &ServiceState) {
        let _ = self.0.try_send(Event::Refresh);
    }
    fn reading(&self, _: &ServiceState, r: &Reading) {
        let _ = self.0.send(Event::Reading(r.clone()));
    }
    fn device(&self, _: &ServiceState, d: &DeviceState) {
        let _ = self.0.send(Event::Device(d.clone()));
    }
    fn relay(&self, _: &ServiceState, r: Option<&RelayInfo>) {
        let _ = self.0.send(Event::Relay(r.copied()));
    }
    fn nfc(&self, _: &ServiceState, n: Option<&NfcInfo>) {
        let _ = self.0.send(Event::Nfc(n.cloned()));
    }
    fn gps(&self, _: &ServiceState, g: Option<&GpsInfo>) {
        let _ = self.0.send(Event::Gps(g.cloned()));
    }
    fn alert(&self, _: &ServiceState, ev: &AlertEvent) {
        let _ = self.0.send(Event::Alert(ev.clone()));
    }
    fn mqtt(&self, _: &ServiceState, m: &MqttState) {
        let _ = self.0.send(Event::Mqtt(m.clone()));
    }
}

/// Everything shown at one moment.
#[derive(Clone, Debug, Serialize)]
pub struct Snapshot {
    pub device: DeviceState,
    pub latest: Vec<Reading>,
    pub relay: Option<RelayInfo>,
    pub nfc: Option<NfcInfo>,
    pub gps: Option<GpsInfo>,
    pub active_alerts: Vec<ActiveAlert>,
    /// `None` when not known (a short session publishes nothing).
    pub mqtt: Option<MqttState>,
    pub sample_interval_ms: u64,
    #[serde(skip)]
    pub unit: TemperatureUnit,
}

pub enum Access {
    /// The service runs in this process.
    Local(Service),
    /// Another instance holds the bridge.
    Remote(Remote),
}

impl Access {
    /// Reach the bridge as described at the top. `observer` gets the
    /// service's reports when it runs here.
    pub fn open(ctx: &Ctx, mode: Mode, observer: Arc<dyn Observer>) -> Result<Self> {
        let paths = ctx.paths()?;
        if let Some(holder) = InstanceLock::holder(&paths.lock_file()) {
            let settings = Settings::read(&paths.settings_file());
            if !settings.api.enabled {
                return Err(crate::failure(
                    exit::BUSY,
                    format!("{holder} is running and holds the bridge"),
                    Some(
                        "turn on its API server (in the app: Settings, API & MCP, Enable API server; for `run`: `config set api.enabled true` while it is stopped), or stop it",
                    ),
                ));
            }
            return Ok(Access::Remote(Remote::new(
                &settings,
                holder,
                paths.settings_file(),
            )));
        }
        match Service::start(ServiceConfig {
            paths,
            mode,
            simulated: ctx.simulate,
            program: "tower-bridge-cli".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            observer,
        }) {
            Ok(service) => Ok(Access::Local(service)),
            // Started in the meantime.
            Err(StartError::AlreadyRunning(holder, _)) => Err(crate::failure(
                exit::BUSY,
                format!("{holder} is running and holds the bridge"),
                Some("try again, or stop it"),
            )),
            Err(e) => Err(e.into()),
        }
    }

    /// Where the bridge is reached, for messages.
    pub fn describe(&self) -> String {
        match self {
            Access::Local(_) => "this process".into(),
            Access::Remote(r) => format!("{} at {}", r.holder, r.url()),
        }
    }

    pub fn snapshot(&self) -> Result<Snapshot> {
        match self {
            Access::Local(s) => {
                let settings = s.settings();
                Ok(Snapshot {
                    device: s.device.read().clone(),
                    latest: s.latest.read().clone(),
                    relay: *s.relay.read(),
                    nfc: s.nfc.read().clone(),
                    gps: s.gps.read().clone(),
                    active_alerts: s.alerts.read().clone(),
                    mqtt: (s.mode == Mode::Full).then(|| s.mqtt.read().clone()),
                    sample_interval_ms: settings.sample_interval_ms,
                    unit: settings.temperature_unit,
                })
            }
            Access::Remote(r) => {
                let status = r.status()?;
                // The unit is a display setting: ours, read again each time.
                let settings = Settings::read(&r.settings_file);
                Ok(Snapshot {
                    device: status.device,
                    latest: r.latest()?,
                    relay: status.relay,
                    nfc: status.nfc,
                    gps: r.gps()?,
                    active_alerts: status.active_alerts,
                    mqtt: status.mqtt,
                    sample_interval_ms: status.sample_interval_ms,
                    unit: settings.temperature_unit,
                })
            }
        }
    }

    /// Wait in a short session until the bridge is found and every channel
    /// with sensors has a reading, or it turns out there is nothing to find.
    /// A running instance answers at once.
    pub fn settle(&self, timeout: Duration) -> Result<Snapshot> {
        if matches!(self, Access::Remote(_)) {
            // A service that has just started may still be looking for the
            // bridge.
            let started = Instant::now();
            loop {
                let snap = self.snapshot()?;
                if !matches!(snap.device, DeviceState::Searching) || started.elapsed() > timeout {
                    return Ok(snap);
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        }
        let started = Instant::now();
        let mut ready_since: Option<Instant> = None;
        loop {
            let snap = self.snapshot()?;
            let done = match &snap.device {
                DeviceState::Ready { sources, .. } => {
                    let since = *ready_since.get_or_insert_with(Instant::now);
                    let mut channels: Vec<u8> = sources.iter().map(|s| s.channel).collect();
                    channels.dedup();
                    let measured = channels
                        .iter()
                        .all(|c| snap.latest.iter().any(|r| r.channel == *c));
                    // A module only measuring later (CO2 after a minute) has no
                    // reading yet; do not wait for it beyond a cycle or two.
                    measured || since.elapsed() > Duration::from_secs(6)
                }
                DeviceState::NoModule { .. } => true,
                DeviceState::Error { kind, .. } => {
                    // Not found: the first attempt may race the bridge's start.
                    *kind != DeviceErrorKind::NotFound || started.elapsed() > Duration::from_secs(3)
                }
                DeviceState::Searching => false,
            };
            if done || started.elapsed() > timeout {
                return Ok(snap);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    pub fn history(&self, ctx: &Ctx, from: i64, to: i64, max_points: usize) -> Result<History> {
        match self {
            Access::Remote(r) => r.history(from, to, max_points),
            Access::Local(s) => {
                local_history(ctx, from, to, max_points, s.settings().sample_interval_ms)
            }
        }
    }

    pub fn identify(&self) -> Result<()> {
        match self {
            Access::Local(s) => s.identify().map_err(acq),
            Access::Remote(r) => r.identify(),
        }
    }

    pub fn rescan(&self) -> Result<()> {
        match self {
            Access::Local(s) => s.rescan().map(|_| ()).map_err(acq),
            Access::Remote(r) => r.rescan(),
        }
    }

    pub fn set_relay(&self, state: bool) -> Result<RelayInfo> {
        match self {
            Access::Local(s) => s.set_relay(state).map_err(acq),
            Access::Remote(r) => r.set_relay(state),
        }
    }

    pub fn pulse_relay(&self, state: Option<bool>, ms: Option<u64>) -> Result<RelayInfo> {
        match self {
            Access::Local(s) => s
                .pulse_relay(state, ms.map(Duration::from_millis))
                .map_err(acq),
            Access::Remote(r) => r.pulse_relay(state, ms),
        }
    }

    pub fn write_nfc(&self, url: &str) -> Result<NfcInfo> {
        match self {
            Access::Local(s) => s.write_nfc_url(url.to_string()).map_err(acq),
            Access::Remote(r) => r.write_nfc(url),
        }
    }

    /// Stop a service running here (the sensors' state is saved).
    pub fn close(&self) {
        if let Access::Local(service) = self {
            service.shutdown(Duration::from_secs(3));
        }
    }
}

/// History straight from the database, which the app and the CLI share.
pub fn local_history(
    ctx: &Ctx,
    from: i64,
    to: i64,
    max_points: usize,
    interval_ms: u64,
) -> Result<History> {
    let conn = store::open_reader(&ctx.paths()?.db_file())?;
    let query = HistoryQuery {
        from,
        to,
        max_points,
        sample_interval_ms: i64::try_from(interval_ms).unwrap_or(i64::MAX),
    };
    Ok(store::history(&conn, &query)?)
}

/// A command for the bridge failed: why, with the matching exit code.
fn acq(e: AcqError) -> anyhow::Error {
    match e {
        AcqError::NotConnected => crate::failure(
            exit::NOT_FOUND,
            "the TOWER Bridge Module is not connected",
            None,
        ),
        AcqError::NoRelay => {
            crate::failure(exit::NOT_FOUND, "no TOWER Relay Module is connected", None)
        }
        AcqError::NoNfc => crate::failure(exit::NOT_FOUND, "no TOWER NFC Tag is connected", None),
        AcqError::Nfc(m) | AcqError::Bus(m) => anyhow::anyhow!(m),
    }
}

/// The exit code and message for a bridge that cannot be used.
pub fn device_failure(d: &DeviceState) -> Option<anyhow::Error> {
    match d {
        DeviceState::Error { kind, message } => Some(match kind {
            DeviceErrorKind::NotFound => crate::failure(
                exit::NOT_FOUND,
                "no TOWER Bridge Module found",
                Some(
                    "check the USB cable; `tower-bridge-cli toolbox scan` lists what the computer sees",
                ),
            ),
            DeviceErrorKind::Busy => crate::failure(
                exit::BUSY,
                "the TOWER Bridge Module is in use by another program",
                Some("quit HARDWARIO Playground, enmon or another TOWER Bridge"),
            ),
            DeviceErrorKind::PermissionDenied => crate::failure(
                exit::ERROR,
                format!("no permission to open the TOWER Bridge Module: {message}"),
                Some("install the udev rule (see docs/cli.md) and plug the bridge in again"),
            ),
            DeviceErrorKind::I2cDevNotLoaded => crate::failure(
                exit::ERROR,
                "the i2c-dev kernel module is not loaded",
                Some("sudo modprobe i2c-dev"),
            ),
            DeviceErrorKind::Failed => crate::failure(exit::ERROR, message.clone(), None),
        }),
        DeviceState::Searching => Some(crate::failure(
            exit::NOT_FOUND,
            "the TOWER Bridge Module did not answer in time",
            None,
        )),
        _ => None,
    }
}

/// A channel of the service's reports, and its observer.
pub fn events() -> (Arc<dyn Observer>, Receiver<Event>) {
    let (tx, rx) = crossbeam_channel::unbounded();
    (Arc::new(Forward(tx)), rx)
}
