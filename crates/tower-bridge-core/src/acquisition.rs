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
use crate::climate::{ChannelMode, ClimateModule};
use crate::model::{ClimateRevision, DeviceState, Reading};
use crate::relay::{MAX_PULSE, MIN_PULSE, RelayInfo, RelayModule};

pub const RECONNECT_INTERVAL: Duration = Duration::from_secs(2);
pub const NO_MODULE_RETRY: Duration = Duration::from_secs(2);
pub const MIN_INTERVAL: Duration = Duration::from_secs(2);
const FAILED_CYCLES_REDETECT: u32 = 3;
const FAILED_CYCLES_REOPEN: u32 = 5;
/// How often to look for a Relay Module that was not found.
const RELAY_PROBE_INTERVAL: Duration = Duration::from_secs(60);
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
    #[error("{0}")]
    Bus(String),
}

/// Result of re-detecting the Climate Module.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RescanOutcome {
    Found {
        revision: ClimateRevision,
        channel: u8,
    },
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
    /// Forget the detected module and detect again right away; replies with
    /// the detection result.
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
    Shutdown,
}

#[derive(Clone, Debug)]
pub enum AcqEvent {
    Device(DeviceState),
    /// A completed measurement (`altitude` is not filled in here).
    Reading(Reading),
    /// Relay Module found, switched or gone (`None`).
    Relay(Option<RelayInfo>),
}

#[derive(Clone, Debug)]
pub struct AcqConfig {
    pub interval: Duration,
    pub channel: ChannelMode,
}

impl Default for AcqConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(10),
            channel: ChannelMode::Auto,
        }
    }
}

/// Opens a bridge. Called again after every disconnect.
pub type Opener = Box<dyn FnMut() -> Result<Box<dyn I2cBus>, OpenError> + Send>;

/// Opener for real hardware.
pub fn hardware_opener(pref: BackendPref) -> Opener {
    let mut discovery = Discovery::new(pref);
    Box::new(move || discovery.open_first())
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
    module: &'a mut ClimateModule,
    relay: &'a mut Option<RelayModule>,
}

struct Worker {
    config: AcqConfig,
    clock: Arc<dyn Clock>,
    rx: Receiver<AcqCommand>,
    events: Sender<AcqEvent>,
    last_state: Option<DeviceState>,
    last_relay: Option<RelayInfo>,
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

    fn emit_relay(&mut self, relay: Option<RelayInfo>) {
        if self.last_relay != relay {
            self.last_relay = relay;
            let _ = self.events.send(AcqEvent::Relay(relay));
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
    fn relay_command(
        &mut self,
        bus: &mut dyn I2cBus,
        relay: &mut Option<RelayModule>,
        cmd: AcqCommand,
    ) -> bool {
        match cmd {
            AcqCommand::SetRelay(state, reply) => {
                self.pulse = None;
                let result = self.move_relay(bus, relay, state);
                if let Some(reply) = reply {
                    let _ = reply.send(result);
                }
                true
            }
            AcqCommand::PulseRelay {
                state,
                duration,
                reply,
            } => {
                let result = self.start_pulse(bus, relay, state, duration);
                if let Some(reply) = reply {
                    let _ = reply.send(result);
                }
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
            match opener() {
                Ok(bus) => match self.run_connected(bus) {
                    Exit::Shutdown => {
                        self.pulse = None;
                        self.emit_relay(None);
                        return;
                    }
                    Exit::Lost(e) => {
                        if self.pulse.take().is_some() {
                            warn!(
                                "bridge lost during a relay pulse; the relay stays switched over"
                            );
                        }
                        self.emit_relay(None);
                        warn!("bridge lost: {e}");
                        self.emit_state(DeviceState::Searching);
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

    fn run_connected(&mut self, mut bus: Box<dyn I2cBus>) -> Exit {
        let bus = bus.as_mut();
        let device = bus.info().clone();
        let _ = self.blink(bus, 1);
        let mut module = ClimateModule::new(self.config.channel);
        let mut relay: Option<RelayModule> = None;
        let mut relay_probed = self.clock.now();
        let mut failed_cycles = 0u32;
        let mut deadline: Option<Duration> = None;

        loop {
            if module.detection().is_none() {
                if relay.is_none() {
                    if let Err(e) = self.probe_relay(bus, &mut relay) {
                        return Exit::Lost(e);
                    }
                    relay_probed = self.clock.now();
                }
                match module.detect(bus, self.clock.as_ref()) {
                    Ok(Some(d)) => {
                        failed_cycles = 0;
                        deadline = None;
                        self.resolve_rescan(Ok(RescanOutcome::Found {
                            revision: d.revision,
                            channel: d.channel,
                        }));
                        self.emit_state(DeviceState::Ready {
                            device: device.clone(),
                            revision: d.revision,
                            channel: d.channel,
                            sources: crate::climate::sources(d.revision),
                        });
                    }
                    Ok(None) => {
                        self.resolve_rescan(Ok(RescanOutcome::NoModule));
                        self.emit_state(DeviceState::NoModule {
                            device: device.clone(),
                        });
                        let session = Session {
                            bus: &mut *bus,
                            module: &mut module,
                            relay: &mut relay,
                        };
                        match self.wait(NO_MODULE_RETRY, Some(session)) {
                            Flow::Shutdown => return self.close(bus),
                            Flow::Elapsed | Flow::Now => continue,
                        }
                    }
                    Err(e) => return Exit::Lost(e),
                }
            }

            if let Some(d) = deadline {
                let session = Session {
                    bus: &mut *bus,
                    module: &mut module,
                    relay: &mut relay,
                };
                match self.wait(d, Some(session)) {
                    Flow::Shutdown => return self.close(bus),
                    Flow::Elapsed | Flow::Now => {}
                }
                if module.detection().is_none() {
                    continue;
                }
            }

            if relay.is_none() && self.clock.now() - relay_probed >= RELAY_PROBE_INTERVAL {
                if let Err(e) = self.probe_relay(bus, &mut relay) {
                    return Exit::Lost(e);
                }
                relay_probed = self.clock.now();
            }

            // Let a relay pulse that ends during the cycle end first.
            if let Some(p) = self.pulse {
                let left = p.due.saturating_duration_since(Instant::now());
                if left < PULSE_GUARD {
                    let session = Session {
                        bus: &mut *bus,
                        module: &mut module,
                        relay: &mut relay,
                    };
                    if self.wait(left, Some(session)) == Flow::Shutdown {
                        return self.close(bus);
                    }
                    if module.detection().is_none() {
                        continue;
                    }
                }
            }

            let ts = unix_ms();
            let clock = Arc::clone(&self.clock);
            let measured = module.measure_with(bus, clock.as_ref(), &mut |bus, until| {
                self.measure_idle(bus, &mut relay, until)
            });
            match measured {
                Ok(report) => {
                    for (sensor, err) in &report.errors {
                        warn!("{}: {err}", sensor.name());
                    }
                    if report.values.is_empty() {
                        failed_cycles += 1;
                    } else {
                        failed_cycles = 0;
                        let _ = self.events.send(AcqEvent::Reading(Reading {
                            ts,
                            device: device.serial.clone(),
                            values: report.values,
                            altitude: None,
                        }));
                    }
                }
                Err(e) => return Exit::Lost(e),
            }
            if failed_cycles >= FAILED_CYCLES_REOPEN {
                return Exit::Lost(BusError::Protocol("no sensor responds".into()));
            }
            if failed_cycles == FAILED_CYCLES_REDETECT {
                let _ = bus.recover();
                module.forget();
            }
            deadline = Some(until_next_tick(self.config.interval));
        }
    }

    fn close(&mut self, bus: &mut dyn I2cBus) -> Exit {
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
            if now >= deadline {
                return Flow::Elapsed;
            }
            let until = pulse_due.map_or(deadline, |due| due.min(deadline));
            let cmd = match self.deferred.pop_front() {
                Some(cmd) => cmd,
                None => match self.rx.recv_timeout(until.saturating_duration_since(now)) {
                    Ok(cmd) => cmd,
                    Err(RecvTimeoutError::Timeout) => continue,
                    Err(RecvTimeoutError::Disconnected) => return Flow::Shutdown,
                },
            };
            match cmd {
                AcqCommand::Shutdown => return Flow::Shutdown,
                AcqCommand::SetInterval(i) => {
                    self.config.interval = i.max(MIN_INTERVAL);
                    if target.is_some() {
                        // Re-align to the new interval.
                        return self.wait(until_next_tick(self.config.interval), target);
                    }
                }
                AcqCommand::SetChannel(mode) => {
                    self.config.channel = mode;
                    if let Some(s) = target.as_mut() {
                        s.module.set_mode(mode);
                        return Flow::Now;
                    }
                }
                AcqCommand::Rescan(reply) => {
                    // Answered after the next detection (or failed open).
                    self.pending_rescan.extend(reply);
                    if let Some(s) = target.as_mut() {
                        s.module.forget();
                        // Looked for again together with the Climate Module
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
                AcqCommand::SetRelay(state, reply) => {
                    let result = match target.as_mut() {
                        Some(s) => {
                            self.pulse = None;
                            self.move_relay(&mut *s.bus, s.relay, state)
                        }
                        None => Err(AcqError::NotConnected),
                    };
                    if let Some(reply) = reply {
                        let _ = reply.send(result);
                    }
                }
                AcqCommand::PulseRelay {
                    state,
                    duration,
                    reply,
                } => {
                    let result = match target.as_mut() {
                        Some(s) => self.start_pulse(&mut *s.bus, s.relay, state, duration),
                        None => Err(AcqError::NotConnected),
                    };
                    if let Some(reply) = reply {
                        let _ = reply.send(result);
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
    use crate::model::{ClimateRevision, DeviceErrorKind, Values};

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
        assert_eq!(
            done.recv_timeout(Duration::from_secs(5)),
            Ok(Ok(RescanOutcome::Found {
                revision: ClimateRevision::R1,
                channel: 1
            }))
        );
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
}
