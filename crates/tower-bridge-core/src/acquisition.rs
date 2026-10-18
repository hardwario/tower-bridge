//! Background acquisition thread: owns the bridge, reconnects when it goes
//! away, and measures on wall-clock aligned ticks.

use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, unbounded};
use serde::Serialize;
use tracing::{error, info, warn};

use crate::bus::discovery::{BackendPref, Discovery, OpenError};
use crate::bus::{BusError, Clock, I2cBus, SystemClock};
use crate::climate::select_channel;
use crate::climate::{ChannelMode, Sensors};
use crate::gps::{GpsInfo, GpsModule};
use crate::model::{DeviceInfo, DeviceState, Reading, SensorSource};
use crate::nfc::{NfcError, NfcInfo, NfcTag};
use crate::relay::{MAX_PULSE, MIN_PULSE, RelayInfo, RelayModule};

pub const RECONNECT_INTERVAL: Duration = Duration::from_secs(2);
pub const NO_MODULE_RETRY: Duration = Duration::from_secs(2);
pub const MIN_INTERVAL: Duration = Duration::from_secs(2);
const FAILED_CYCLES_REDETECT: u32 = 3;
const FAILED_CYCLES_REOPEN: u32 = 5;
/// How often to look for sensors and a Relay Module that were not found
/// (a tag may be plugged in while the app runs).
const PROBE_INTERVAL: Duration = Duration::from_secs(60);
/// Switch channels searched for a Relay Module.
const RELAY_CHANNELS: [u8; 2] = [0, 1];
/// A measurement cycle takes about a second; one is not started when a relay
/// pulse ends sooner than this, so the pulse ends on time.
const PULSE_GUARD: Duration = Duration::from_millis(1500);
/// How often relay commands are checked while a measurement waits for its
/// sensors.
const IDLE_SLICE: Duration = Duration::from_millis(25);

/// Why a command could not be carried out.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error, Serialize)]
#[serde(tag = "code", content = "message", rename_all = "snake_case")]
pub enum AcqError {
    #[error("TOWER Bridge is not connected")]
    NotConnected,
    #[error("Relay Module not found")]
    NoRelay,
    #[error("TOWER NFC Tag not found")]
    NoNfc,
    #[error("{0}")]
    Nfc(String),
    #[error("{0}")]
    Bus(String),
}

/// Result of looking for the sensors again.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RescanOutcome {
    Found { sources: Vec<SensorSource> },
    NoModule,
}

/// Where a command reports its outcome.
pub type Reply<T> = Sender<Result<T, AcqError>>;

#[derive(Clone, Debug)]
pub enum AcqCommand {
    SetInterval(Duration),
    SetChannel(ChannelMode),
    /// Blink the bridge LED; replies when the blinking has finished.
    Identify(Option<Reply<()>>),
    /// Forget the detected sensors and look again right away; replies with
    /// the result.
    Rescan(Option<Reply<RescanOutcome>>),
    /// Switch the Relay Module to `true` or `false`; ends a
    /// running pulse. Replies with its new state.
    SetRelay(bool, Option<Reply<RelayInfo>>),
    /// Switch the relay to `state` for `duration`, then back. Without a
    /// state: the opposite of the current one (`false` when unknown), or the
    /// running pulse is extended. Replies once the relay has switched over.
    PulseRelay {
        state: Option<bool>,
        duration: Duration,
        reply: Option<Reply<RelayInfo>>,
    },
    /// Write a URL to the NFC Tag (between measurements); replies with what
    /// the tag holds afterwards.
    WriteNfcUrl(String, Option<Reply<NfcInfo>>),
    Shutdown,
}

impl AcqCommand {
    fn is_relay(&self) -> bool {
        matches!(
            self,
            AcqCommand::SetRelay(..) | AcqCommand::PulseRelay { .. }
        )
    }

    fn take_relay_reply(&mut self) -> Option<Reply<RelayInfo>> {
        match self {
            AcqCommand::SetRelay(_, reply) | AcqCommand::PulseRelay { reply, .. } => reply.take(),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub enum AcqEvent {
    Device(DeviceState),
    /// A completed measurement of one channel (`altitude` is not filled in
    /// here). Channels measured in the same cycle share the timestamp.
    Reading(Reading),
    /// Relay Module found, switched or gone (`None`).
    Relay(Option<RelayInfo>),
    /// NFC Tag found, written, its URL changed, or gone (`None`).
    Nfc(Option<NfcInfo>),
    /// GPS Module found, its position or fix changed, or gone (`None`).
    Gps(Option<GpsInfo>),
}

#[derive(Clone, Debug)]
pub struct AcqConfig {
    pub interval: Duration,
    pub channel: ChannelMode,
    /// Where sensors keep state between runs (the LP8 sensor state, the
    /// SGPC3 baseline); none keeps nothing.
    pub state_dir: Option<std::path::PathBuf>,
}

impl Default for AcqConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(10),
            channel: ChannelMode::Auto,
            state_dir: None,
        }
    }
}

/// Opens a bridge (again after every disconnect) and counts the bridges.
pub trait Bridges: Send {
    fn open(&mut self) -> Result<Box<dyn I2cBus>, OpenError>;

    /// TOWER Bridge Modules plugged in, the open one included.
    fn count(&mut self) -> usize {
        1
    }
}

/// A closure that opens a bridge, with one bridge plugged in.
impl<F> Bridges for F
where
    F: FnMut() -> Result<Box<dyn I2cBus>, OpenError> + Send,
{
    fn open(&mut self) -> Result<Box<dyn I2cBus>, OpenError> {
        self()
    }
}

pub type Opener = Box<dyn Bridges>;

struct Hardware(Discovery);

impl Bridges for Hardware {
    fn open(&mut self) -> Result<Box<dyn I2cBus>, OpenError> {
        self.0.open_first()
    }

    fn count(&mut self) -> usize {
        self.0.count()
    }
}

/// Opener for real hardware.
pub fn hardware_opener(pref: BackendPref) -> Opener {
    Box::new(Hardware(Discovery::new(pref)))
}

/// Opener that yields a simulated bridge (development without hardware).
pub fn simulated_opener() -> Opener {
    Box::new(|| Ok(Box::new(crate::bus::mock::SimulatedBridge::new()) as Box<dyn I2cBus>))
}

pub struct Acquisition {
    tx: Sender<AcqCommand>,
    join: Option<JoinHandle<()>>,
}

impl Acquisition {
    pub fn spawn(
        config: AcqConfig,
        opener: Opener,
        clock: Box<dyn Clock>,
        events: Sender<AcqEvent>,
    ) -> std::io::Result<Self> {
        let (tx, rx) = unbounded();
        let join = std::thread::Builder::new()
            .name("acquisition".into())
            .spawn(move || supervise(config, opener, clock, rx, events))?;
        Ok(Self {
            tx,
            join: Some(join),
        })
    }

    /// Spawn against real hardware.
    pub fn spawn_hardware(
        config: AcqConfig,
        pref: BackendPref,
        events: Sender<AcqEvent>,
    ) -> std::io::Result<Self> {
        Self::spawn(
            config,
            hardware_opener(pref),
            Box::new(SystemClock::new()),
            events,
        )
    }

    pub fn sender(&self) -> Sender<AcqCommand> {
        self.tx.clone()
    }

    /// Stop the thread (turns the LED off and closes the device), waiting up to `timeout`.
    pub fn shutdown(mut self, timeout: Duration) {
        let _ = self.tx.send(AcqCommand::Shutdown);
        if let Some(join) = self.join.take() {
            let deadline = Instant::now() + timeout;
            while !join.is_finished() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            if join.is_finished() {
                let _ = join.join();
            }
        }
    }
}

fn supervise(
    config: AcqConfig,
    mut opener: Opener,
    clock: Box<dyn Clock>,
    rx: Receiver<AcqCommand>,
    events: Sender<AcqEvent>,
) {
    let mut worker = Worker {
        config,
        clock: Arc::from(clock),
        rx,
        events,
        last_state: None,
        last_relay: None,
        last_nfc: None,
        last_gps: None,
        gps_rejected_until: [None; 2],
        pulse: None,
        deferred: VecDeque::new(),
        pending_rescan: Vec::new(),
    };
    loop {
        let r = catch_unwind(AssertUnwindSafe(|| worker.run(&mut opener)));
        match r {
            Ok(()) => break,
            Err(_) => {
                error!("acquisition thread panicked; restarting");
                worker.last_state = None;
                if worker.wait(RECONNECT_INTERVAL, None) == Flow::Shutdown {
                    break;
                }
            }
        }
    }
    info!("acquisition stopped");
}

#[derive(Debug, PartialEq, Eq)]
enum Flow {
    /// The wait ended normally.
    Elapsed,
    /// Measure immediately.
    Now,
    Shutdown,
    /// The bridge failed while serving a background job.
    Lost(BusError),
}

enum Exit {
    Shutdown,
    Lost(BusError),
}

/// A running timed relay pulse.
#[derive(Clone, Copy, Debug)]
struct PendingPulse {
    due: Instant,
    /// Unix ms of `due`, for reporting.
    until_ms: i64,
    back: bool,
}

/// What a wait on a connected bridge can act on.
struct Session<'a> {
    bus: &'a mut dyn I2cBus,
    sensors: &'a mut Sensors,
    relay: &'a mut Option<RelayModule>,
    nfc: &'a mut Option<NfcTag>,
    gps: &'a mut Option<GpsModule>,
    /// Waiting for the next measurement tick (a new interval re-aligns it).
    ticking: bool,
}

struct Worker {
    config: AcqConfig,
    clock: Arc<dyn Clock>,
    rx: Receiver<AcqCommand>,
    events: Sender<AcqEvent>,
    last_state: Option<DeviceState>,
    last_relay: Option<RelayInfo>,
    last_nfc: Option<NfcInfo>,
    last_gps: Option<GpsInfo>,
    /// Per channel, the clock time until which no GPS Module is looked for
    /// there (something else answered at its expander address).
    gps_rejected_until: [Option<Duration>; 2],
    pulse: Option<PendingPulse>,
    /// Commands received during a measurement that wait for its end.
    deferred: VecDeque<AcqCommand>,
    pending_rescan: Vec<Reply<RescanOutcome>>,
}

impl Worker {
    fn resolve_rescan(&mut self, result: Result<RescanOutcome, AcqError>) {
        for reply in self.pending_rescan.drain(..) {
            let _ = reply.send(result.clone());
        }
    }

    fn emit_state(&mut self, state: DeviceState) {
        if self.last_state.as_ref() != Some(&state) {
            self.last_state = Some(state.clone());
            let _ = self.events.send(AcqEvent::Device(state));
        }
    }

    /// Report the sensors found (they changed, or someone asked).
    fn report_sensors(&mut self, device: &DeviceInfo, sensors: &Sensors) {
        let sources = sensors.sources();
        self.resolve_rescan(Ok(RescanOutcome::Found {
            sources: sources.clone(),
        }));
        self.emit_state(DeviceState::Ready {
            device: device.clone(),
            sources,
        });
    }

    /// Count the bridges plugged in; a change is reported with the device.
    fn recount(&mut self, opener: &mut Opener, device: &mut DeviceInfo) {
        let n = u32::try_from(opener.count().max(1)).unwrap_or(u32::MAX);
        if n == device.bridges_connected {
            return;
        }
        if n > 1 {
            warn!(
                "{n} TOWER Bridge Modules are connected; only {} is read",
                device.path
            );
        }
        device.bridges_connected = n;
        if let Some(mut state) = self.last_state.clone()
            && let DeviceState::Ready { device: d, .. } | DeviceState::NoModule { device: d } =
                &mut state
        {
            d.clone_from(device);
            self.emit_state(state);
        }
    }

    fn emit_relay(&mut self, relay: Option<RelayInfo>) {
        if self.last_relay != relay {
            self.last_relay = relay;
            let _ = self.events.send(AcqEvent::Relay(relay));
        }
    }

    fn emit_nfc(&mut self, nfc: Option<NfcInfo>) {
        if self.last_nfc != nfc {
            match (&self.last_nfc, &nfc) {
                (None, Some(n)) => info!(
                    "TOWER NFC Tag ({}) found on I2C channel {}",
                    n.part, n.channel
                ),
                (Some(_), None) => info!("TOWER NFC Tag gone"),
                _ => {}
            }
            self.last_nfc = nfc.clone();
            let _ = self.events.send(AcqEvent::Nfc(nfc));
        }
    }

    fn emit_gps(&mut self, gps: Option<GpsInfo>) {
        if self.last_gps != gps {
            if self.last_gps.is_none() != gps.is_none() {
                match &gps {
                    Some(g) => info!("TOWER GPS Module found on I2C channel {}", g.channel),
                    None => info!("TOWER GPS Module gone"),
                }
            }
            self.last_gps = gps.clone();
            let _ = self.events.send(AcqEvent::Gps(gps));
        }
    }

    /// Look for a GPS Module on either channel and power its receiver.
    fn probe_gps(
        &mut self,
        bus: &mut dyn I2cBus,
        gps: &mut Option<GpsModule>,
    ) -> Result<(), BusError> {
        if gps.is_some() {
            return Ok(());
        }
        let now = self.clock.now();
        for channel in RELAY_CHANNELS {
            if self.gps_rejected_until[usize::from(channel)].is_some_and(|t| now < t) {
                continue;
            }
            let found = select_channel(bus, channel).and_then(|()| GpsModule::probe(bus));
            match found {
                Ok(true) => {}
                Ok(false) => continue,
                Err(e) if e.is_fatal() => return Err(e),
                Err(_) => continue,
            }
            match GpsModule::start(bus, self.clock.as_ref(), channel) {
                Ok(module) => {
                    *gps = Some(module);
                    return Ok(());
                }
                Err(e) if e.is_fatal() => return Err(e),
                Err(_) => continue,
            }
        }
        Ok(())
    }

    /// Between measurements: TVOC every 30 s, the GPS position every 5 s.
    /// Every job served moves its next time on, also when it fails; `Err`
    /// is a lost bridge.
    fn serve_background(&mut self, s: &mut Session<'_>) -> Result<(), BusError> {
        let clock = Arc::clone(&self.clock);
        // Only transport failures come back; sensor failures are counted.
        s.sensors.service(&mut *s.bus, clock.as_ref())?;
        let Some(module) = s.gps.as_mut() else {
            return Ok(());
        };
        if !module.due(clock.now()) {
            return Ok(());
        }
        let changed = match select_channel(&mut *s.bus, module.channel()) {
            Ok(()) => module.service(&mut *s.bus, clock.as_ref())?,
            Err(e) if e.is_fatal() => return Err(e),
            Err(_) => {
                module.missed(clock.now());
                false
            }
        };
        if module.gone() {
            // Not a GPS Module after all (or unplugged): leave it alone a while.
            let channel = module.channel();
            warn!("no GPS receiver answers on channel {channel}");
            let _ = module.release(&mut *s.bus);
            *s.gps = None;
            // Another channel may still have one.
            self.gps_rejected_until[usize::from(channel) % 2] =
                Some(clock.now() + Duration::from_secs(600));
            self.emit_gps(None);
        } else if changed || self.last_gps.is_none() {
            let info = module.info().cloned();
            if info.is_some() {
                self.emit_gps(info);
            }
        }
        Ok(())
    }

    /// Clock time of the next background job.
    fn background_due(s: &Session<'_>) -> Option<Duration> {
        let gps = s.gps.as_ref().map(GpsModule::next);
        match (s.sensors.next_service(), gps) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// Look for the NFC Tag again; it also catches a URL a phone wrote.
    fn probe_nfc(
        &mut self,
        bus: &mut dyn I2cBus,
        nfc: &mut Option<NfcTag>,
    ) -> Result<(), BusError> {
        *nfc = NfcTag::detect(bus, &RELAY_CHANNELS)?;
        self.emit_nfc(nfc.as_ref().map(NfcTag::info));
        Ok(())
    }

    fn write_nfc(&mut self, s: &mut Session<'_>, url: &str) -> Result<NfcInfo, AcqError> {
        let tag = s.nfc.as_mut().ok_or(AcqError::NoNfc)?;
        let result = tag.write_url(&mut *s.bus, self.clock.as_ref(), url);
        let info = tag.info();
        self.emit_nfc(Some(info.clone()));
        match result {
            Ok(()) => {
                info!("TOWER NFC Tag URL written");
                Ok(info)
            }
            Err(NfcError::Bus(e)) => Err(AcqError::Bus(e.to_string())),
            Err(e) => Err(AcqError::Nfc(e.to_string())),
        }
    }

    fn relay_info(&self, relay: &RelayModule) -> RelayInfo {
        RelayInfo {
            pulse_until: self.pulse.map(|p| p.until_ms),
            ..relay.info()
        }
    }

    /// Look for a Relay Module (transport failures propagate).
    fn probe_relay(
        &mut self,
        bus: &mut dyn I2cBus,
        relay: &mut Option<RelayModule>,
    ) -> Result<(), BusError> {
        *relay = RelayModule::detect(bus, &RELAY_CHANNELS)?;
        self.pulse = None;
        let info = relay.as_ref().map(|r| self.relay_info(r));
        self.emit_relay(info);
        Ok(())
    }

    /// Switch the relay and report its state; a relay that stopped
    /// answering is forgotten.
    fn move_relay(
        &mut self,
        bus: &mut dyn I2cBus,
        relay: &mut Option<RelayModule>,
        state: bool,
    ) -> Result<RelayInfo, AcqError> {
        self.switch_relay(bus, relay, state)?;
        let info = relay
            .as_ref()
            .map(|r| self.relay_info(r))
            .ok_or(AcqError::NoRelay)?;
        self.emit_relay(Some(info));
        Ok(info)
    }

    /// One coil pulse; a relay that stopped answering is forgotten.
    fn switch_relay(
        &mut self,
        bus: &mut dyn I2cBus,
        relay: &mut Option<RelayModule>,
        state: bool,
    ) -> Result<(), AcqError> {
        let Some(module) = relay.as_mut() else {
            self.pulse = None;
            return Err(AcqError::NoRelay);
        };
        module.set(bus, self.clock.as_ref(), state).map_err(|e| {
            if e == BusError::AddrNack {
                *relay = None;
                self.pulse = None;
                self.emit_relay(None);
            }
            AcqError::Bus(e.to_string())
        })
    }

    fn start_pulse(
        &mut self,
        bus: &mut dyn I2cBus,
        relay: &mut Option<RelayModule>,
        state: Option<bool>,
        duration: Duration,
    ) -> Result<RelayInfo, AcqError> {
        let current = relay.as_ref().ok_or(AcqError::NoRelay)?.state();
        let running = self.pulse.map(|p| !p.back);
        let target = state
            .or(running)
            .unwrap_or_else(|| current.is_some_and(|c| !c));
        // Extending a pulse keeps where it returns to.
        let back = match self.pulse {
            Some(p) if running == Some(target) => p.back,
            _ => !target,
        };
        if !(running == Some(target) && current == Some(target)) {
            self.pulse = None;
            self.switch_relay(bus, relay, target)?;
        }
        // Timed from the moment the relay has switched (switching may first
        // wait for the coil capacitor to recharge).
        let duration = duration.clamp(MIN_PULSE, MAX_PULSE);
        self.pulse = Some(PendingPulse {
            due: Instant::now() + duration,
            until_ms: unix_ms() + duration.as_millis() as i64,
            back,
        });
        info!("relay pulse {target} for {} ms", duration.as_millis());
        let info = relay
            .as_ref()
            .map(|r| self.relay_info(r))
            .ok_or(AcqError::NoRelay)?;
        self.emit_relay(Some(info));
        Ok(info)
    }

    fn finish_pulse(&mut self, bus: &mut dyn I2cBus, relay: &mut Option<RelayModule>) {
        let Some(p) = self.pulse.take() else {
            return;
        };
        if let Err(e) = self.move_relay(bus, relay, p.back) {
            warn!("relay pulse could not switch back: {e}");
        }
    }

    /// Relay commands are carried out at once; anything else waits until
    /// the end of the measurement. Returns whether the bus was used.
    /// The last of the relay commands queued behind `cmd` (a relay command)
    /// takes its place. A burst of commands (a looping automation, many API
    /// calls at once) then switches the relay once rather than once per
    /// command, at 300 ms or more each, long after the callers gave up. The
    /// replies of the replaced commands come back to get the same answer.
    fn latest_relay_command(&mut self, mut cmd: AcqCommand) -> (AcqCommand, Vec<Reply<RelayInfo>>) {
        while let Ok(queued) = self.rx.try_recv() {
            self.deferred.push_back(queued);
        }
        let mut replaced = Vec::new();
        while let Some(i) = self.deferred.iter().position(AcqCommand::is_relay) {
            let Some(next) = self.deferred.remove(i) else {
                break;
            };
            replaced.extend(cmd.take_relay_reply());
            cmd = next;
        }
        (cmd, replaced)
    }

    fn relay_command(
        &mut self,
        bus: &mut dyn I2cBus,
        relay: &mut Option<RelayModule>,
        cmd: AcqCommand,
    ) -> bool {
        let (cmd, replaced) = if cmd.is_relay() {
            self.latest_relay_command(cmd)
        } else {
            (cmd, Vec::new())
        };
        let answer = |reply: Option<Reply<RelayInfo>>, result: Result<RelayInfo, AcqError>| {
            for r in replaced.iter().chain(reply.as_ref()) {
                let _ = r.send(result.clone());
            }
        };
        match cmd {
            AcqCommand::SetRelay(state, reply) => {
                self.pulse = None;
                let result = self.move_relay(bus, relay, state);
                answer(reply, result);
                true
            }
            AcqCommand::PulseRelay {
                state,
                duration,
                reply,
            } => {
                let result = self.start_pulse(bus, relay, state, duration);
                answer(reply, result);
                true
            }
            other => {
                self.deferred.push_back(other);
                false
            }
        }
    }

    /// While a measurement waits for its sensors (until `until`, clock time),
    /// keep the relay responsive: switch on request and end pulses on time.
    fn measure_idle(
        &mut self,
        bus: &mut dyn I2cBus,
        relay: &mut Option<RelayModule>,
        until: Duration,
    ) -> bool {
        let mut used = false;
        loop {
            if self.pulse.is_some_and(|p| p.due <= Instant::now()) {
                self.finish_pulse(bus, relay);
                used = true;
            }
            while let Ok(cmd) = self.rx.try_recv() {
                used |= self.relay_command(bus, relay, cmd);
            }
            let now = self.clock.now();
            if now >= until {
                return used;
            }
            self.clock.sleep((until - now).min(IDLE_SLICE));
        }
    }

    fn run(&mut self, opener: &mut Opener) {
        loop {
            if self.last_state.is_none() {
                self.emit_state(DeviceState::Searching);
            }
            match opener.open() {
                Ok(bus) => match self.run_connected(bus, opener) {
                    Exit::Shutdown => {
                        self.pulse = None;
                        self.emit_relay(None);
                        self.emit_nfc(None);
                        self.emit_gps(None);
                        return;
                    }
                    Exit::Lost(e) => {
                        if self.pulse.take().is_some() {
                            warn!(
                                "bridge lost during a relay pulse; the relay stays switched over"
                            );
                        }
                        self.emit_relay(None);
                        self.emit_nfc(None);
                        self.emit_gps(None);
                        warn!("bridge lost: {e}");
                        self.emit_state(DeviceState::Searching);
                        // A bridge that opens but fails right away must not
                        // be reopened in a tight loop; commands are served.
                        if self.wait(RECONNECT_INTERVAL, None) == Flow::Shutdown {
                            return;
                        }
                    }
                },
                Err(e) => {
                    self.emit_state(DeviceState::Error {
                        kind: e.kind,
                        message: e.message,
                    });
                    self.resolve_rescan(Err(AcqError::NotConnected));
                    if self.wait(RECONNECT_INTERVAL, None) == Flow::Shutdown {
                        return;
                    }
                }
            }
        }
    }

    fn run_connected(&mut self, mut bus: Box<dyn I2cBus>, opener: &mut Opener) -> Exit {
        let bus = bus.as_mut();
        let mut device = bus.info().clone();
        self.recount(opener, &mut device);
        let _ = self.blink(bus, 1);
        let mut sensors =
            Sensors::new(self.config.channel).with_state_dir(self.config.state_dir.clone());
        let mut gps: Option<GpsModule> = None;
        let mut relay: Option<RelayModule> = None;
        let mut nfc: Option<NfcTag> = None;
        let mut probed = self.clock.now();
        let mut failed_cycles = 0u32;
        let mut deadline: Option<Duration> = None;

        loop {
            if sensors.is_empty() {
                if relay.is_none()
                    && let Err(e) = self.probe_relay(bus, &mut relay)
                {
                    return Exit::Lost(e);
                }
                if let Err(e) = self.probe_nfc(bus, &mut nfc) {
                    return Exit::Lost(e);
                }
                if let Err(e) = self.probe_gps(bus, &mut gps) {
                    return Exit::Lost(e);
                }
                probed = self.clock.now();
                self.recount(opener, &mut device);
                match sensors.detect(bus, self.clock.as_ref()) {
                    // Failed cycles keep counting: sensors that are found
                    // again but still fail end in a reopen.
                    Ok(_) if !sensors.is_empty() => {
                        deadline = None;
                        self.report_sensors(&device, &sensors);
                    }
                    Ok(_) => {
                        self.resolve_rescan(Ok(RescanOutcome::NoModule));
                        self.emit_state(DeviceState::NoModule {
                            device: device.clone(),
                        });
                        let session = Session {
                            bus: &mut *bus,
                            sensors: &mut sensors,
                            relay: &mut relay,
                            nfc: &mut nfc,
                            gps: &mut gps,
                            ticking: false,
                        };
                        match self.wait(NO_MODULE_RETRY, Some(session)) {
                            Flow::Shutdown => return self.close(bus, &sensors, gps.as_ref()),
                            Flow::Lost(e) => return Exit::Lost(e),
                            Flow::Elapsed | Flow::Now => continue,
                        }
                    }
                    Err(e) => return Exit::Lost(e),
                }
            }

            if let Some(d) = deadline {
                let session = Session {
                    bus: &mut *bus,
                    sensors: &mut sensors,
                    relay: &mut relay,
                    nfc: &mut nfc,
                    gps: &mut gps,
                    ticking: true,
                };
                match self.wait(d, Some(session)) {
                    Flow::Shutdown => return self.close(bus, &sensors, gps.as_ref()),
                    Flow::Lost(e) => return Exit::Lost(e),
                    Flow::Elapsed | Flow::Now => {}
                }
                if sensors.is_empty() {
                    continue;
                }
            }

            if self.clock.now() - probed >= PROBE_INTERVAL {
                self.recount(opener, &mut device);
                if relay.is_none()
                    && let Err(e) = self.probe_relay(bus, &mut relay)
                {
                    return Exit::Lost(e);
                }
                if let Err(e) = self.probe_nfc(bus, &mut nfc) {
                    return Exit::Lost(e);
                }
                if let Err(e) = self.probe_gps(bus, &mut gps) {
                    return Exit::Lost(e);
                }
                match sensors.detect(bus, self.clock.as_ref()) {
                    Ok(true) => self.report_sensors(&device, &sensors),
                    Ok(false) => {}
                    Err(e) => return Exit::Lost(e),
                }
                probed = self.clock.now();
            }

            // Let a relay pulse that ends during the cycle end first.
            if let Some(p) = self.pulse {
                let left = p.due.saturating_duration_since(Instant::now());
                if left < PULSE_GUARD {
                    let session = Session {
                        bus: &mut *bus,
                        sensors: &mut sensors,
                        relay: &mut relay,
                        nfc: &mut nfc,
                        gps: &mut gps,
                        ticking: false,
                    };
                    match self.wait(left, Some(session)) {
                        Flow::Shutdown => return self.close(bus, &sensors, gps.as_ref()),
                        Flow::Lost(e) => return Exit::Lost(e),
                        Flow::Elapsed | Flow::Now => {}
                    }
                    if sensors.is_empty() {
                        continue;
                    }
                }
            }

            let ts = unix_ms();
            let clock = Arc::clone(&self.clock);
            let measured = sensors.measure_with(bus, clock.as_ref(), &mut |bus, until| {
                self.measure_idle(bus, &mut relay, until)
            });
            match measured {
                Ok(reports) => {
                    let mut measured_any = false;
                    let mut errors = 0;
                    for (channel, report) in reports {
                        errors += report.errors.len() + usize::from(report.unreachable);
                        for (sensor, err) in &report.errors {
                            warn!("{} on channel {channel}: {err}", sensor.name());
                        }
                        if !report.values.is_empty() {
                            measured_any = true;
                            let _ = self.events.send(AcqEvent::Reading(Reading {
                                ts,
                                device: device.serial.clone(),
                                channel,
                                values: report.values,
                                altitude: None,
                            }));
                        }
                    }
                    // No value is a failure only when parts or the switch
                    // said so: a CO2 Module still charging or a VOC-LP Tag
                    // still warming up has nothing to report yet.
                    if measured_any || errors == 0 {
                        failed_cycles = 0;
                    } else {
                        failed_cycles += 1;
                    }
                }
                Err(e) => return Exit::Lost(e),
            }
            // Parts that stopped answering were dropped, here or between
            // the cycles (a VOC-LP Tag served in the background).
            if !sensors.is_empty()
                && self.last_state.as_ref().map(DeviceState::sources)
                    != Some(&sensors.sources()[..])
            {
                self.report_sensors(&device, &sensors);
            }
            if failed_cycles >= FAILED_CYCLES_REOPEN {
                return Exit::Lost(BusError::Protocol("no sensor responds".into()));
            }
            if failed_cycles == FAILED_CYCLES_REDETECT {
                let _ = bus.recover();
                sensors.forget();
            }
            deadline = Some(until_next_tick(self.config.interval));
        }
    }

    fn close(&mut self, bus: &mut dyn I2cBus, sensors: &Sensors, gps: Option<&GpsModule>) -> Exit {
        sensors.stop(bus);
        if let Some(gps) = gps {
            let _ = gps.stop(bus);
        }
        let _ = bus.set_led(false);
        Exit::Shutdown
    }

    fn blink(&self, bus: &mut dyn I2cBus, times: usize) -> Result<(), BusError> {
        for i in 0..times {
            bus.set_led(true)?;
            self.clock.sleep(Duration::from_millis(150));
            bus.set_led(false)?;
            if i + 1 < times {
                self.clock.sleep(Duration::from_millis(150));
            }
        }
        Ok(())
    }

    /// Wait for `d` while handling commands and ending relay pulses on time.
    fn wait(&mut self, d: Duration, mut target: Option<Session<'_>>) -> Flow {
        let deadline = Instant::now() + d;
        loop {
            let now = Instant::now();
            let pulse_due = self.pulse.filter(|_| target.is_some()).map(|p| p.due);
            if let (Some(due), Some(s)) = (pulse_due, target.as_mut())
                && due <= now
            {
                self.finish_pulse(&mut *s.bus, s.relay);
                continue;
            }
            // The measurement first: a background job due at the same time
            // is served in the next wait.
            if now >= deadline {
                return Flow::Elapsed;
            }
            let background = target
                .as_ref()
                .and_then(Self::background_due)
                .map(|due| now + due.saturating_sub(self.clock.now()));
            if let (Some(due), Some(s)) = (background, target.as_mut())
                && due <= now
            {
                if let Err(e) = self.serve_background(s) {
                    return Flow::Lost(e);
                }
                continue;
            }
            let until = [pulse_due, background]
                .into_iter()
                .flatten()
                .fold(deadline, Instant::min);
            let cmd = match self.deferred.pop_front() {
                Some(cmd) => cmd,
                None => match self.rx.recv_timeout(until.saturating_duration_since(now)) {
                    Ok(cmd) => cmd,
                    Err(RecvTimeoutError::Timeout) => continue,
                    Err(RecvTimeoutError::Disconnected) => return Flow::Shutdown,
                },
            };
            if cmd.is_relay()
                && let Some(s) = target.as_mut()
            {
                self.relay_command(&mut *s.bus, s.relay, cmd);
                continue;
            }
            match cmd {
                AcqCommand::Shutdown => return Flow::Shutdown,
                AcqCommand::SetInterval(i) => {
                    self.config.interval = i.max(MIN_INTERVAL);
                    if target.as_ref().is_some_and(|t| t.ticking) {
                        // Re-align to the new interval.
                        return self.wait(until_next_tick(self.config.interval), target);
                    }
                }
                AcqCommand::SetChannel(mode) => {
                    self.config.channel = mode;
                    if let Some(s) = target.as_mut() {
                        s.sensors.set_mode(mode);
                        return Flow::Now;
                    }
                }
                AcqCommand::Rescan(reply) => {
                    // Answered after the next detection (or failed open).
                    self.pending_rescan.extend(reply);
                    // A GPS Module plugged in where something else was.
                    self.gps_rejected_until = [None; 2];
                    if let Some(s) = target.as_mut() {
                        s.sensors.forget();
                        // Looked for again together with the sensors
                        // (unless a pulse still has to switch it back).
                        if self.pulse.is_none() {
                            *s.relay = None;
                        }
                    }
                    return Flow::Now;
                }
                AcqCommand::Identify(reply) => {
                    let result = match target.as_mut() {
                        Some(s) => self
                            .blink(&mut *s.bus, 3)
                            .map_err(|e| AcqError::Bus(e.to_string())),
                        None => Err(AcqError::NotConnected),
                    };
                    if let Some(reply) = reply {
                        let _ = reply.send(result);
                    }
                }
                AcqCommand::WriteNfcUrl(url, reply) => {
                    let result = match target.as_mut() {
                        Some(s) => self.write_nfc(s, &url),
                        None => Err(AcqError::NotConnected),
                    };
                    if let Some(reply) = reply {
                        let _ = reply.send(result);
                    }
                }
                // Served above while connected.
                AcqCommand::SetRelay(_, reply) | AcqCommand::PulseRelay { reply, .. } => {
                    if let Some(reply) = reply {
                        let _ = reply.send(Err(AcqError::NotConnected));
                    }
                }
            }
        }
    }
}

pub fn unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Time until the next wall-clock multiple of `interval`.
fn until_next_tick(interval: Duration) -> Duration {
    let interval_ms = interval.as_millis().max(1) as i64;
    let now = unix_ms();
    let next = (now / interval_ms + 1) * interval_ms;
    Duration::from_millis((next - now) as u64)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::bus::mock::{MockBus, MockClock};
    use crate::model::{ClimateRevision, DeviceErrorKind, ModuleKind, Values};

    fn recv_until<F: Fn(&AcqEvent) -> bool>(rx: &Receiver<AcqEvent>, f: F) -> Option<AcqEvent> {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if let Ok(ev) = rx.recv_timeout(Duration::from_millis(100))
                && f(&ev)
            {
                return Some(ev);
            }
        }
        None
    }

    #[test]
    fn measures_and_shuts_down() {
        let env = Arc::new(Mutex::new(Values {
            temperature: Some(21.0),
            humidity: Some(50.0),
            illuminance: Some(100.0),
            pressure: Some(100_000.0),
            co2: None,
            voc: None,
        }));
        let opener: Opener = Box::new(move || {
            Ok(
                Box::new(MockBus::with_climate(ClimateRevision::R2, 0, env.clone()))
                    as Box<dyn I2cBus>,
            )
        });
        let (tx, rx) = unbounded();
        let acq = match Acquisition::spawn(
            AcqConfig::default(),
            opener,
            Box::new(MockClock::new()),
            tx,
        ) {
            Ok(a) => a,
            Err(e) => panic!("spawn failed: {e}"),
        };
        let ready = recv_until(&rx, |e| {
            matches!(e, AcqEvent::Device(DeviceState::Ready { .. }))
        });
        assert!(ready.is_some());
        let reading = recv_until(&rx, |e| matches!(e, AcqEvent::Reading(_)));
        match reading {
            Some(AcqEvent::Reading(r)) => {
                assert_eq!(r.device, "SIMULATED");
                assert!(r.values.temperature.is_some());
            }
            other => panic!("unexpected {other:?}"),
        }
        acq.shutdown(Duration::from_secs(2));
    }

    #[test]
    fn both_channels_are_measured_in_one_cycle() {
        let indoor = Arc::new(Mutex::new(Values {
            temperature: Some(22.0),
            ..Values::default()
        }));
        let outdoor = Arc::new(Mutex::new(Values {
            temperature: Some(4.0),
            humidity: Some(85.0),
            ..Values::default()
        }));
        let opener: Opener = Box::new(move || {
            let mut bus = MockBus::with_climate(ClimateRevision::R1, 0, indoor.clone());
            bus.attach(
                1,
                0x40,
                Box::new(crate::bus::mock::SimSht20::new(outdoor.clone())),
            );
            Ok(Box::new(bus) as Box<dyn I2cBus>)
        });
        let (tx, rx) = unbounded();
        let acq = match Acquisition::spawn(
            AcqConfig::default(),
            opener,
            Box::new(MockClock::new()),
            tx,
        ) {
            Ok(a) => a,
            Err(e) => panic!("spawn failed: {e}"),
        };
        let ready = recv_until(&rx, |e| {
            matches!(e, AcqEvent::Device(DeviceState::Ready { .. }))
        });
        let Some(AcqEvent::Device(DeviceState::Ready { sources, .. })) = ready else {
            panic!("not ready: {ready:?}");
        };
        assert_eq!(
            crate::model::modules(&sources),
            vec![
                (0, ModuleKind::ClimateModuleR1),
                (1, ModuleKind::HumidityTag)
            ]
        );
        let mut readings = Vec::new();
        while readings.len() < 2 {
            match recv_until(&rx, |e| matches!(e, AcqEvent::Reading(_))) {
                Some(AcqEvent::Reading(r)) => readings.push(r),
                other => panic!("unexpected {other:?}"),
            }
        }
        assert_eq!(
            readings.iter().map(|r| r.channel).collect::<Vec<_>>(),
            vec![0, 1]
        );
        assert_eq!(readings[0].ts, readings[1].ts, "one cycle, one timestamp");
        let t1 = readings[1].values.temperature.unwrap_or_default();
        assert!((t1 - 4.0).abs() < 0.1, "the tag's thermometer: {t1}");
        acq.shutdown(Duration::from_secs(2));
    }

    #[test]
    fn a_bridge_that_fails_at_once_is_retried_slowly_and_still_shuts_down() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let opened = Arc::new(AtomicUsize::new(0));
        let count = opened.clone();
        let opener: Opener = Box::new(move || {
            count.fetch_add(1, Ordering::Relaxed);
            let mut bus = MockBus::new();
            bus.inject = vec![BusError::Hid("gone".into()); 1000];
            Ok(Box::new(bus) as Box<dyn I2cBus>)
        });
        let (tx, _rx) = unbounded();
        let acq = match Acquisition::spawn(
            AcqConfig::default(),
            opener,
            Box::new(MockClock::new()),
            tx,
        ) {
            Ok(a) => a,
            Err(e) => panic!("spawn failed: {e}"),
        };
        std::thread::sleep(Duration::from_millis(1000));
        assert_eq!(opened.load(Ordering::Relaxed), 1, "no tight reopen loop");
        let started = Instant::now();
        acq.shutdown(Duration::from_secs(3));
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "shutdown served"
        );
    }

    #[test]
    fn reports_open_errors() {
        let opener: Opener = Box::new(|| {
            Err(OpenError {
                kind: DeviceErrorKind::NotFound,
                message: "none".into(),
            })
        });
        let (tx, rx) = unbounded();
        let acq = match Acquisition::spawn(
            AcqConfig::default(),
            opener,
            Box::new(MockClock::new()),
            tx,
        ) {
            Ok(a) => a,
            Err(e) => panic!("spawn failed: {e}"),
        };
        let ev = recv_until(&rx, |e| {
            matches!(
                e,
                AcqEvent::Device(DeviceState::Error {
                    kind: DeviceErrorKind::NotFound,
                    ..
                })
            )
        });
        assert!(ev.is_some());
        acq.shutdown(Duration::from_secs(2));
    }

    #[test]
    fn identify_and_rescan_reply() {
        let env = Arc::new(Mutex::new(Values::default()));
        let opener: Opener = Box::new(move || {
            Ok(
                Box::new(MockBus::with_climate(ClimateRevision::R1, 1, env.clone()))
                    as Box<dyn I2cBus>,
            )
        });
        let (tx, rx) = unbounded();
        let acq = match Acquisition::spawn(
            AcqConfig::default(),
            opener,
            Box::new(MockClock::new()),
            tx,
        ) {
            Ok(a) => a,
            Err(e) => panic!("spawn failed: {e}"),
        };
        assert!(recv_until(&rx, |e| matches!(e, AcqEvent::Reading(_))).is_some());

        let (reply, done) = crossbeam_channel::bounded(1);
        let _ = acq.sender().send(AcqCommand::Identify(Some(reply)));
        assert_eq!(done.recv_timeout(Duration::from_secs(5)), Ok(Ok(())));

        let (reply, done) = crossbeam_channel::bounded(1);
        let _ = acq.sender().send(AcqCommand::Rescan(Some(reply)));
        match done.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(RescanOutcome::Found { sources })) => assert_eq!(
                crate::model::modules(&sources),
                vec![(1, ModuleKind::ClimateModuleR1)]
            ),
            other => panic!("unexpected {other:?}"),
        }
        acq.shutdown(Duration::from_secs(2));
    }

    #[test]
    fn finds_the_nfc_tag_and_writes_a_url() {
        let opener: Opener = Box::new(move || {
            let env = Arc::new(Mutex::new(Values {
                temperature: Some(21.0),
                ..Values::default()
            }));
            let mut bus = MockBus::with_climate(ClimateRevision::R2, 0, env);
            bus.attach(
                1,
                crate::nfc::ADDRESSES[0],
                Box::new(crate::bus::mock::SimNtag::new()),
            );
            Ok(Box::new(bus) as Box<dyn I2cBus>)
        });
        let (tx, rx) = unbounded();
        let acq = match Acquisition::spawn(
            AcqConfig::default(),
            opener,
            Box::new(MockClock::new()),
            tx,
        ) {
            Ok(a) => a,
            Err(e) => panic!("spawn failed: {e}"),
        };
        let found = recv_until(&rx, |e| matches!(e, AcqEvent::Nfc(Some(_))));
        let Some(AcqEvent::Nfc(Some(info))) = found else {
            panic!("no NFC Tag event");
        };
        assert_eq!((info.channel, info.url.as_deref()), (1, None));
        let (reply, done) = crossbeam_channel::bounded(1);
        let _ = acq.sender().send(AcqCommand::WriteNfcUrl(
            "https://www.hardwario.com".into(),
            Some(reply),
        ));
        match done.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(info)) => assert_eq!(info.url.as_deref(), Some("https://www.hardwario.com")),
            other => panic!("unexpected {other:?}"),
        }
        let (reply, done) = crossbeam_channel::bounded(1);
        let _ = acq
            .sender()
            .send(AcqCommand::WriteNfcUrl(" ".into(), Some(reply)));
        assert!(matches!(
            done.recv_timeout(Duration::from_secs(5)),
            Ok(Err(AcqError::Nfc(_)))
        ));
        acq.shutdown(Duration::from_secs(2));
    }

    #[test]
    fn says_when_more_bridges_are_plugged_in() {
        struct Two;
        impl Bridges for Two {
            fn open(&mut self) -> Result<Box<dyn I2cBus>, OpenError> {
                let env = Arc::new(Mutex::new(Values {
                    temperature: Some(21.0),
                    ..Values::default()
                }));
                Ok(Box::new(MockBus::with_climate(ClimateRevision::R2, 0, env)) as Box<dyn I2cBus>)
            }
            fn count(&mut self) -> usize {
                2
            }
        }
        let (tx, rx) = unbounded();
        let acq = match Acquisition::spawn(
            AcqConfig::default(),
            Box::new(Two),
            Box::new(MockClock::new()),
            tx,
        ) {
            Ok(a) => a,
            Err(e) => panic!("spawn failed: {e}"),
        };
        let ready = recv_until(&rx, |e| {
            matches!(e, AcqEvent::Device(DeviceState::Ready { .. }))
        });
        let Some(AcqEvent::Device(DeviceState::Ready { device, .. })) = ready else {
            panic!("no ready state");
        };
        assert_eq!(device.bridges_connected, 2);
        acq.shutdown(Duration::from_secs(2));
    }

    #[test]
    fn commands_report_not_connected() {
        let opener: Opener = Box::new(|| {
            Err(OpenError {
                kind: DeviceErrorKind::NotFound,
                message: "none".into(),
            })
        });
        let (tx, rx) = unbounded();
        let acq = match Acquisition::spawn(
            AcqConfig::default(),
            opener,
            Box::new(MockClock::new()),
            tx,
        ) {
            Ok(a) => a,
            Err(e) => panic!("spawn failed: {e}"),
        };
        assert!(
            recv_until(&rx, |e| matches!(
                e,
                AcqEvent::Device(DeviceState::Error { .. })
            ))
            .is_some()
        );
        let (reply, done) = crossbeam_channel::bounded(1);
        let _ = acq.sender().send(AcqCommand::Identify(Some(reply)));
        assert_eq!(
            done.recv_timeout(Duration::from_secs(5)),
            Ok(Err(AcqError::NotConnected))
        );
        let (reply, done) = crossbeam_channel::bounded(1);
        let _ = acq.sender().send(AcqCommand::Rescan(Some(reply)));
        assert_eq!(
            done.recv_timeout(Duration::from_secs(5)),
            Ok(Err(AcqError::NotConnected))
        );
        acq.shutdown(Duration::from_secs(2));
    }

    fn spawn_with(relay: bool) -> (Acquisition, Receiver<AcqEvent>) {
        let env = Arc::new(Mutex::new(Values::default()));
        let opener: Opener = Box::new(move || {
            let mut bus = MockBus::with_climate(ClimateRevision::R1, 0, env.clone());
            if relay {
                bus.attach(
                    1,
                    crate::relay::ADDR_DEFAULT,
                    Box::new(crate::bus::mock::SimTca9534a::new()),
                );
            }
            Ok(Box::new(bus) as Box<dyn I2cBus>)
        });
        let (tx, rx) = unbounded();
        match Acquisition::spawn(AcqConfig::default(), opener, Box::new(MockClock::new()), tx) {
            Ok(a) => (a, rx),
            Err(e) => panic!("spawn failed: {e}"),
        }
    }

    #[test]
    fn detects_and_switches_the_relay() {
        let (acq, rx) = spawn_with(true);
        let found = recv_until(&rx, |e| matches!(e, AcqEvent::Relay(Some(_))));
        let Some(AcqEvent::Relay(Some(info))) = found else {
            panic!("relay not reported: {found:?}");
        };
        assert_eq!((info.channel, info.state), (1, None));

        let (reply, done) = crossbeam_channel::bounded(1);
        let _ = acq.sender().send(AcqCommand::SetRelay(false, Some(reply)));
        let switched = done.recv_timeout(Duration::from_secs(5));
        assert!(matches!(
            switched,
            Ok(Ok(RelayInfo {
                state: Some(false),
                ..
            }))
        ));
        assert!(
            recv_until(&rx, |e| matches!(
                e,
                AcqEvent::Relay(Some(RelayInfo {
                    state: Some(false),
                    ..
                }))
            ))
            .is_some()
        );
        acq.shutdown(Duration::from_secs(2));
    }

    #[test]
    fn pulse_switches_over_and_back() {
        let (acq, rx) = spawn_with(true);
        assert!(recv_until(&rx, |e| matches!(e, AcqEvent::Relay(Some(_)))).is_some());
        let started = Instant::now();
        let (reply, done) = crossbeam_channel::bounded(1);
        let _ = acq.sender().send(AcqCommand::PulseRelay {
            state: None,
            duration: MIN_PULSE,
            reply: Some(reply),
        });
        // Unknown state: pulses to false ...
        let first = done.recv_timeout(Duration::from_secs(5));
        match first {
            Ok(Ok(info)) => {
                assert_eq!(info.state, Some(false));
                assert!(info.pulse_until.is_some());
            }
            other => panic!("unexpected {other:?}"),
        }
        // ... and returns to true once the pulse has ended.
        let back = recv_until(&rx, |e| {
            matches!(
                e,
                AcqEvent::Relay(Some(RelayInfo {
                    state: Some(true),
                    pulse_until: None,
                    ..
                }))
            )
        });
        assert!(back.is_some());
        assert!(started.elapsed() >= MIN_PULSE);
        acq.shutdown(Duration::from_secs(2));
    }

    #[test]
    fn a_pulse_right_after_a_switch_lasts_its_full_length() {
        let (acq, rx) = spawn_with(true);
        assert!(recv_until(&rx, |e| matches!(e, AcqEvent::Relay(Some(_)))).is_some());
        let (reply, done) = crossbeam_channel::bounded(1);
        let _ = acq.sender().send(AcqCommand::SetRelay(true, Some(reply)));
        assert!(matches!(
            done.recv_timeout(Duration::from_secs(5)),
            Ok(Ok(_))
        ));
        let (reply, done) = crossbeam_channel::bounded(1);
        let _ = acq.sender().send(AcqCommand::PulseRelay {
            state: Some(false),
            duration: MIN_PULSE,
            reply: Some(reply),
        });
        let started = match done.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(info)) => info.pulse_until.unwrap_or(0),
            other => panic!("unexpected {other:?}"),
        };
        // The reported end is a full pulse after the switch.
        assert!(started - unix_ms() >= MIN_PULSE.as_millis() as i64 - 50);
        acq.shutdown(Duration::from_secs(2));
    }

    #[test]
    fn setting_a_state_ends_a_pulse_early() {
        let (acq, rx) = spawn_with(true);
        assert!(recv_until(&rx, |e| matches!(e, AcqEvent::Relay(Some(_)))).is_some());
        let (reply, done) = crossbeam_channel::bounded(1);
        let _ = acq.sender().send(AcqCommand::PulseRelay {
            state: Some(false),
            duration: Duration::from_secs(60),
            reply: Some(reply),
        });
        assert!(matches!(
            done.recv_timeout(Duration::from_secs(5)),
            Ok(Ok(_))
        ));
        // The dashboard's "end now": move back to where the pulse returns to.
        let (reply, done) = crossbeam_channel::bounded(1);
        let _ = acq.sender().send(AcqCommand::SetRelay(true, Some(reply)));
        match done.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(info)) => {
                assert_eq!(info.state, Some(true));
                assert_eq!(info.pulse_until, None, "the pulse is over");
            }
            other => panic!("unexpected {other:?}"),
        }
        acq.shutdown(Duration::from_secs(2));
    }

    #[test]
    fn relay_commands_without_a_relay() {
        let (acq, rx) = spawn_with(false);
        assert!(
            recv_until(&rx, |e| matches!(
                e,
                AcqEvent::Device(DeviceState::Ready { .. })
            ))
            .is_some()
        );
        let (reply, done) = crossbeam_channel::bounded(1);
        let _ = acq.sender().send(AcqCommand::SetRelay(true, Some(reply)));
        assert_eq!(
            done.recv_timeout(Duration::from_secs(5)),
            Ok(Err(AcqError::NoRelay))
        );
        acq.shutdown(Duration::from_secs(2));
    }

    #[test]
    fn tick_alignment() {
        let d = until_next_tick(Duration::from_secs(10));
        assert!(d <= Duration::from_secs(10));
    }

    /// A clock `scale` times faster than real time, for the CO2 Module's
    /// first minute and the background jobs.
    struct FastClock {
        origin: Instant,
        scale: u32,
    }

    impl Clock for FastClock {
        fn now(&self) -> Duration {
            self.origin.elapsed() * self.scale
        }

        fn sleep(&self, d: Duration) {
            std::thread::sleep(d / self.scale);
        }
    }

    fn fast(scale: u32) -> Box<dyn Clock> {
        Box::new(FastClock {
            origin: Instant::now(),
            scale,
        })
    }

    /// A bus that can be unplugged: every transfer then fails as USB does.
    struct Unpluggable {
        inner: MockBus,
        gone: Arc<std::sync::atomic::AtomicBool>,
    }

    impl Unpluggable {
        fn check(&mut self) -> Result<(), BusError> {
            self.inner.log.clear();
            if self.gone.load(std::sync::atomic::Ordering::Relaxed) {
                Err(BusError::Hid("unplugged".into()))
            } else {
                Ok(())
            }
        }
    }

    impl I2cBus for Unpluggable {
        fn write(&mut self, addr: u8, data: &[u8]) -> Result<(), BusError> {
            self.check()?;
            self.inner.write(addr, data)
        }

        fn read(&mut self, addr: u8, buf: &mut [u8]) -> Result<(), BusError> {
            self.check()?;
            self.inner.read(addr, buf)
        }

        fn write_read(&mut self, addr: u8, wr: &[u8], rd: &mut [u8]) -> Result<(), BusError> {
            self.check()?;
            self.inner.write_read(addr, wr, rd)
        }

        fn recover(&mut self) -> Result<(), BusError> {
            Ok(())
        }

        fn set_led(&mut self, _on: bool) -> Result<(), BusError> {
            self.check()
        }

        fn info(&self) -> &DeviceInfo {
            self.inner.info()
        }
    }

    fn spawn_fast(opener: Opener, scale: u32) -> (Acquisition, Receiver<AcqEvent>) {
        let (tx, rx) = unbounded();
        let config = AcqConfig {
            interval: Duration::from_secs(2),
            ..AcqConfig::default()
        };
        match Acquisition::spawn(config, opener, fast(scale), tx) {
            Ok(a) => (a, rx),
            Err(e) => panic!("spawn failed: {e}"),
        }
    }

    #[test]
    fn unplugging_with_background_jobs_reconnects() {
        use crate::bus::mock::{attach_gps_module, attach_sgpc3};
        let gone = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let g = gone.clone();
        let opener: Opener = Box::new(move || {
            let mut bus = MockBus::with_climate(ClimateRevision::R2, 0, Arc::default());
            attach_sgpc3(&mut bus, 0);
            attach_gps_module(&mut bus, 1);
            Ok(Box::new(Unpluggable {
                inner: bus,
                gone: g.clone(),
            }) as Box<dyn I2cBus>)
        });
        let (acq, rx) = spawn_fast(opener, 100);
        assert!(recv_until(&rx, |e| matches!(e, AcqEvent::Reading(_))).is_some());
        // Unplugged between measurements, with TVOC and GPS jobs due: the
        // loss is noticed instead of the jobs being served again forever.
        gone.store(true, std::sync::atomic::Ordering::Relaxed);
        let searching = recv_until(&rx, |e| {
            matches!(e, AcqEvent::Device(DeviceState::Searching))
        });
        assert!(searching.is_some(), "the lost bridge is noticed");
        let started = Instant::now();
        acq.shutdown(Duration::from_secs(2));
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "shutdown served"
        );
    }

    #[test]
    fn a_co2_module_alone_is_measured_after_its_first_charge() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let opens = Arc::new(AtomicUsize::new(0));
        let count = opens.clone();
        let opener: Opener = Box::new(move || {
            count.fetch_add(1, Ordering::Relaxed);
            let mut bus = MockBus::new();
            crate::bus::mock::attach_co2_module(&mut bus, 0);
            Ok(Box::new(bus) as Box<dyn I2cBus>)
        });
        let (acq, rx) = spawn_fast(opener, 100);
        // The empty cycles of the first minute are no failures.
        let reading = recv_until(
            &rx,
            |e| matches!(e, AcqEvent::Reading(r) if r.values.co2.is_some()),
        );
        assert!(reading.is_some(), "a CO2 value after the first charge");
        assert_eq!(opens.load(Ordering::Relaxed), 1, "never reopened");
        acq.shutdown(Duration::from_secs(2));
    }

    #[test]
    fn a_voc_tag_dropped_between_cycles_leaves_the_sources() {
        use crate::model::Metric;
        let gone = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let g = gone.clone();
        let opener: Opener = Box::new(move || {
            let mut bus = MockBus::with_climate(ClimateRevision::R2, 0, Arc::default());
            let sgp = crate::bus::mock::attach_sgpc3(&mut bus, 1);
            let g = g.clone();
            // The tag stops answering its measurements once `gone` is set.
            std::thread::spawn(move || {
                while !g.load(std::sync::atomic::Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(20));
                }
                if let Ok(mut s) = sgp.lock() {
                    s.mute = true;
                }
            });
            Ok(Box::new(bus) as Box<dyn I2cBus>)
        });
        let (acq, rx) = spawn_fast(opener, 20);
        let with_voc = |e: &AcqEvent, voc: bool| {
            matches!(e, AcqEvent::Device(DeviceState::Ready { sources, .. })
                if sources.iter().any(|s| s.metric == Metric::Voc) == voc)
        };
        assert!(recv_until(&rx, |e| with_voc(e, true)).is_some());
        gone.store(true, std::sync::atomic::Ordering::Relaxed);
        // Three failed measurements, 30 s apart: 4.5 s at 20 times speed.
        let deadline = Instant::now() + Duration::from_secs(12);
        let mut dropped = false;
        while Instant::now() < deadline && !dropped {
            if let Ok(ev) = rx.recv_timeout(Duration::from_millis(100)) {
                dropped = with_voc(&ev, false);
            }
        }
        assert!(dropped, "the sources no longer list the VOC-LP Tag");
        acq.shutdown(Duration::from_secs(2));
    }

    #[test]
    fn a_gps_module_is_found_next_to_another_device_at_its_address() {
        let opener: Opener = Box::new(move || {
            let mut bus = MockBus::with_climate(ClimateRevision::R2, 0, Arc::default());
            // Something else at 0x21 on channel 0, the GPS Module on 1.
            bus.attach(
                0,
                crate::gps::EXPANDER,
                Box::new(crate::bus::mock::SimTca9534a::new()),
            );
            let gps = crate::bus::mock::attach_gps_module(&mut bus, 1);
            if let Ok(mut g) = gps.lock() {
                g.position = Some((50.0, 15.0, 300.0));
                g.satellites = 8;
            }
            Ok(Box::new(bus) as Box<dyn I2cBus>)
        });
        let (acq, rx) = spawn_fast(opener, 100);
        // The waits between cycles run in real time: the foreign expander
        // fails three polls, one per 2 s cycle, before channel 1 is tried.
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut found = None;
        while found.is_none() && Instant::now() < deadline {
            if let Ok(ev) = rx.recv_timeout(Duration::from_millis(100))
                && matches!(&ev, AcqEvent::Gps(Some(g)) if g.fix)
            {
                found = Some(ev);
            }
        }
        match found {
            Some(AcqEvent::Gps(Some(g))) => assert_eq!(g.channel, 1),
            other => panic!("no GPS fix: {other:?}"),
        }
        acq.shutdown(Duration::from_secs(2));
    }

    #[test]
    fn a_burst_of_relay_commands_switches_once_and_answers_all() {
        let env = Arc::new(Mutex::new(Values::default()));
        let opener: Opener = Box::new(move || {
            let mut bus = MockBus::with_climate(ClimateRevision::R1, 0, env.clone());
            bus.attach(
                1,
                crate::relay::ADDR_DEFAULT,
                Box::new(crate::bus::mock::SimTca9534a::new()),
            );
            Ok(Box::new(bus) as Box<dyn I2cBus>)
        });
        let (tx, rx) = unbounded();
        // Real time: each switch waits for the coil capacitor.
        let acq = match Acquisition::spawn(
            AcqConfig::default(),
            opener,
            Box::new(SystemClock::new()),
            tx,
        ) {
            Ok(a) => a,
            Err(e) => panic!("spawn failed: {e}"),
        };
        assert!(recv_until(&rx, |e| matches!(e, AcqEvent::Relay(Some(_)))).is_some());
        let mut replies = Vec::new();
        for i in 0..10 {
            let (reply, done) = crossbeam_channel::bounded(1);
            let _ = acq
                .sender()
                .send(AcqCommand::SetRelay(i % 2 == 0, Some(reply)));
            replies.push(done);
        }
        for done in &replies {
            let answer = done.recv_timeout(Duration::from_secs(5));
            assert!(
                matches!(answer, Ok(Ok(_))),
                "every caller is answered: {answer:?}"
            );
        }
        let last = replies[9].try_recv();
        assert!(last.is_err(), "one answer each");
        // The last command decides, and the relay switched far fewer times
        // than it was told to.
        std::thread::sleep(Duration::from_millis(200));
        let switches = rx
            .try_iter()
            .filter(|e| matches!(e, AcqEvent::Relay(Some(r)) if r.state.is_some()))
            .count();
        assert!(switches <= 3, "{switches} switches for 10 commands");
        acq.shutdown(Duration::from_secs(2));
    }

    /// The switch stops answering (a module holding the bus): every channel
    /// is unreachable, which must count as failed cycles and recover the bus.
    struct StuckSwitch {
        inner: MockBus,
        stuck: Arc<std::sync::atomic::AtomicBool>,
        recovers: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl I2cBus for StuckSwitch {
        fn write(&mut self, addr: u8, data: &[u8]) -> Result<(), BusError> {
            if addr == crate::climate::SWITCH_ADDR
                && self.stuck.load(std::sync::atomic::Ordering::Relaxed)
            {
                return Err(BusError::Timeout);
            }
            self.inner.write(addr, data)
        }

        fn read(&mut self, addr: u8, buf: &mut [u8]) -> Result<(), BusError> {
            self.inner.read(addr, buf)
        }

        fn write_read(&mut self, addr: u8, wr: &[u8], rd: &mut [u8]) -> Result<(), BusError> {
            self.inner.write_read(addr, wr, rd)
        }

        fn recover(&mut self) -> Result<(), BusError> {
            self.recovers
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(())
        }

        fn info(&self) -> &DeviceInfo {
            self.inner.info()
        }
    }

    #[test]
    fn a_stuck_switch_is_recovered() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        let stuck = Arc::new(AtomicBool::new(false));
        let recovers = Arc::new(AtomicUsize::new(0));
        let (s, r) = (stuck.clone(), recovers.clone());
        let opener: Opener = Box::new(move || {
            Ok(Box::new(StuckSwitch {
                inner: MockBus::with_climate(ClimateRevision::R2, 0, Arc::default()),
                stuck: s.clone(),
                recovers: r.clone(),
            }) as Box<dyn I2cBus>)
        });
        let (acq, rx) = spawn_fast(opener, 1);
        assert!(recv_until(&rx, |e| matches!(e, AcqEvent::Reading(_))).is_some());
        stuck.store(true, Ordering::Relaxed);
        // Three failed cycles, 2 s apart.
        let deadline = Instant::now() + Duration::from_secs(14);
        while recovers.load(Ordering::Relaxed) == 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
        }
        let n = recovers.load(Ordering::Relaxed);
        acq.shutdown(Duration::from_secs(2));
        assert!(n > 0, "the bus was never recovered");
    }
}
