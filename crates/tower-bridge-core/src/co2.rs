//! TOWER CO2 Module: a Senseair LP8 behind an SC16IS740 UART bridge (0x4D),
//! with its supply, capacitor charging, enable and RDY line on a TCA9534A
//! (0x38).
//!
//! Unlike twr-sdk, which charges the capacitor for 5 s before every
//! measurement and switches everything off afterwards (battery devices), the
//! bridge is powered over USB: after the first charge the capacitor stays on
//! charge, so a measurement can start at any time. The sensor state the LP8
//! hands back after each measurement is written back with the next one and
//! stored, so its noise filters and automatic baseline correction (ABC)
//! survive an app restart. The value reported is the filtered, pressure
//! compensated one (the datasheet specifies its accuracy for it), with the
//! pressure measured by a barometer on the bridge when there is one.

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tracing::{debug, info, warn};

use crate::bus::{BusError, Clock, I2cBus};
use crate::climate::select_channel;
use crate::sensors::lp8::{self, Calculation, FrameError, Measurement};
use crate::sensors::sc16is740;

pub const EXPANDER: u8 = 0x38;
pub const UART: u8 = sc16is740::ADDR;

const REG_INPUT: u8 = 0x00;
const REG_OUTPUT: u8 = 0x01;
const REG_CONFIG: u8 = 0x03;

// The output latch stays 0: a pin set as output pulls its line low (on), an
// input lets the pull-up switch it off. P0 and P4 are always outputs.
const IDLE: u8 = 0xEE;
/// Capacitor charging (P1) and the sensor supply VDD2 (P2) on.
const CHARGE: u8 = IDLE & !0x02 & !0x04;
/// And the sensor enabled (P3).
const ENABLE: u8 = CHARGE & !0x08;
/// UART bridge reset (P6, active low).
const UART_RESET: u8 = 0x40;
/// RDY from the LP8 (P7, input).
const RDY: u8 = 0x80;

/// The first charge of a possibly empty capacitor (twr-sdk waits a minute).
pub const FIRST_CHARGE: Duration = Duration::from_secs(60);
/// Shortest time between measurements: below it the residual heat of the
/// lamp spoils the accuracy.
pub const MIN_PERIOD: Duration = Duration::from_secs(16);
/// How often the host asks for an automatic baseline correction.
pub const ABC_PERIOD: Duration = Duration::from_secs(8 * 24 * 3600);
/// After a failed measurement, try again after this long.
const RETRY: Duration = Duration::from_secs(30);
/// A stored sensor state older than this is not reused.
const STATE_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 3600);
/// Failed measurements in a row after which the module counts as gone.
const GIVE_UP: u8 = 3;

// Measurement timing, from the datasheet's table (25 °C, FW 1.08 and later,
// with margin for the bridge's slow polling): RDY low 136 to 191 ms after
// power on, RDY high up to 372 ms after power on (calibrations).
const BOOT_WAIT: Duration = Duration::from_millis(150);
const RDY_LOW_BY: Duration = Duration::from_millis(500);
const RDY_HIGH_BY: Duration = Duration::from_millis(1000);
const ACK_WITHIN: Duration = Duration::from_millis(300);
const REPLY_WITHIN: Duration = Duration::from_millis(400);
const POLL: Duration = Duration::from_millis(10);

#[derive(Debug, thiserror::Error)]
pub enum Co2Error {
    #[error(transparent)]
    Bus(#[from] BusError),
    #[error("LP8 did not {0} in time")]
    Timeout(&'static str),
    #[error("LP8 reply: {0}")]
    Frame(#[from] FrameError),
}

impl Co2Error {
    pub fn is_fatal(&self) -> bool {
        matches!(self, Co2Error::Bus(e) if e.is_fatal())
    }
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// Waits that may lend the bus to others (a relay command); returns whether
/// the bus was used, so the channel must be selected again.
pub type Idle<'a> = &'a mut dyn FnMut(&mut dyn I2cBus, Duration) -> bool;

#[derive(Clone, Debug)]
pub struct Co2Module {
    channel: u8,
    /// From the previous measurement, for the next one.
    state: Option<[u8; lp8::STATE_LEN]>,
    /// Unix ms of the next ABC.
    abc_due: u64,
    /// Clock time of the next measurement.
    next: Duration,
    failures: u8,
    /// The last good value (ppm) and its clock time.
    value: Option<(f64, Duration)>,
    last: Option<Measurement>,
    store: Option<PathBuf>,
}

impl Co2Module {
    /// Whether the module answers on the selected channel: its expander and
    /// its UART bridge.
    pub fn probe(bus: &mut dyn I2cBus) -> Result<bool, BusError> {
        let mut config = [0u8];
        match bus.write_read(EXPANDER, &[REG_CONFIG], &mut config) {
            Ok(()) => {}
            Err(e) if e.is_fatal() => return Err(e),
            Err(_) => return Ok(false),
        }
        match sc16is740::present(bus, UART) {
            Ok(found) => Ok(found),
            Err(e) if e.is_fatal() => Err(e),
            Err(_) => Ok(false),
        }
    }

    /// Set up a module found on `channel` (selected) and start charging.
    /// `store` keeps the sensor state between runs.
    pub fn start(
        bus: &mut dyn I2cBus,
        clock: &dyn Clock,
        channel: u8,
        store: Option<PathBuf>,
    ) -> Result<Self, BusError> {
        bus.write(EXPANDER, &[REG_OUTPUT, 0x00])?;
        // Reset the UART bridge (the pulse needs > 3 µs, the start > 10 µs).
        bus.write(EXPANDER, &[REG_CONFIG, IDLE & !UART_RESET])?;
        bus.write(EXPANDER, &[REG_CONFIG, IDLE])?;
        clock.sleep(Duration::from_millis(1));
        sc16is740::init(bus, UART)?;
        bus.write(EXPANDER, &[REG_CONFIG, CHARGE])?;
        let now = clock.now();
        let mut module = Self {
            channel,
            state: None,
            abc_due: unix_ms() + u64::try_from(ABC_PERIOD.as_millis()).unwrap_or(0),
            next: now + FIRST_CHARGE,
            failures: 0,
            value: None,
            last: None,
            store,
        };
        module.load();
        Ok(module)
    }

    pub fn channel(&self) -> u8 {
        self.channel
    }

    /// Whether a measurement should run now.
    pub fn due(&self, now: Duration) -> bool {
        now >= self.next
    }

    /// The last good value, if measured within `max_age`.
    pub fn value(&self, now: Duration, max_age: Duration) -> Option<f64> {
        self.value
            .filter(|(_, at)| now.saturating_sub(*at) <= max_age)
            .map(|(v, _)| v)
    }

    pub fn last(&self) -> Option<&Measurement> {
        self.last.as_ref()
    }

    /// Too many failures in a row: the module seems gone.
    pub fn gone(&self) -> bool {
        self.failures >= GIVE_UP
    }

    /// Measure once (the channel must be selected). `pressure_pa` is the
    /// ambient pressure for compensation, if known. Switches the sensor off
    /// again in any case.
    pub fn measure(
        &mut self,
        bus: &mut dyn I2cBus,
        clock: &dyn Clock,
        idle: Idle<'_>,
        pressure_pa: Option<f64>,
    ) -> Result<Option<f64>, Co2Error> {
        let calculation = match self.state {
            None => Calculation::Initial,
            Some(_) if unix_ms() >= self.abc_due => Calculation::Abc,
            Some(_) => Calculation::Sequential,
        };
        let pressure = pressure_pa
            .map(|pa| (pa / 10.0).round())
            .filter(|p| (3000.0..=12000.0).contains(p))
            .map_or(lp8::DEFAULT_PRESSURE, |p| p as u16);
        let started = clock.now();
        let result = self.cycle(bus, clock, idle, calculation, pressure);
        // Off again (the capacitor keeps charging), whatever happened.
        let off = bus.write(EXPANDER, &[REG_CONFIG, CHARGE]);
        self.next = started + MIN_PERIOD;
        let m = match result.and_then(|m| off.map(|()| m).map_err(Co2Error::from)) {
            Ok(m) => m,
            Err(e) => {
                self.failures = self.failures.saturating_add(1);
                self.next = started + RETRY;
                return Err(e);
            }
        };
        self.failures = 0;
        debug!(
            "LP8: {:?}, {} ppm (filtered {}, unfiltered {} / {}), {:.2} °C, VCAP {:.2} / {:.2} V, status {:02x?}",
            calculation,
            m.filtered_pressure,
            m.filtered,
            m.co2_pressure,
            m.co2,
            m.temperature,
            m.vcap1,
            m.vcap2,
            m.status
        );
        if m.needs_restart() {
            warn!(
                "LP8 reported a fault (status {:02x?}); starting over",
                m.status
            );
            self.state = None;
        } else {
            self.state = Some(m.state);
            if matches!(calculation, Calculation::Initial | Calculation::Abc) {
                self.abc_due = unix_ms() + u64::try_from(ABC_PERIOD.as_millis()).unwrap_or(0);
                if calculation == Calculation::Abc {
                    info!(
                        "LP8 automatic baseline correction done (status {:02x?})",
                        m.status
                    );
                }
            }
        }
        if m.vcap_low() {
            warn!(
                "LP8 capacitor too low (VCAP {:.2} / {:.2} V); charging longer",
                m.vcap1, m.vcap2
            );
            self.next = started + RETRY;
        }
        let ppm = m.ppm();
        if let Some(v) = ppm {
            self.value = Some((v, clock.now()));
        }
        self.last = Some(m);
        self.save();
        Ok(ppm)
    }

    fn pause(
        &self,
        bus: &mut dyn I2cBus,
        clock: &dyn Clock,
        idle: Idle<'_>,
        until: Duration,
    ) -> Result<(), BusError> {
        if until > clock.now() && idle(bus, until) {
            select_channel(bus, self.channel)?;
        }
        Ok(())
    }

    fn rdy(&self, bus: &mut dyn I2cBus) -> Result<bool, BusError> {
        let mut input = [0u8];
        bus.write_read(EXPANDER, &[REG_INPUT], &mut input)?;
        Ok(input[0] & RDY != 0)
    }

    fn wait_rdy(
        &self,
        bus: &mut dyn I2cBus,
        clock: &dyn Clock,
        idle: Idle<'_>,
        high: bool,
        by: Duration,
        what: &'static str,
    ) -> Result<(), Co2Error> {
        loop {
            if self.rdy(bus)? == high {
                return Ok(());
            }
            let now = clock.now();
            if now >= by {
                return Err(Co2Error::Timeout(what));
            }
            self.pause(bus, clock, idle, (now + POLL).min(by))?;
        }
    }

    fn receive(
        &self,
        bus: &mut dyn I2cBus,
        clock: &dyn Clock,
        idle: Idle<'_>,
        len: usize,
        within: Duration,
        what: &'static str,
    ) -> Result<Vec<u8>, Co2Error> {
        let by = clock.now() + within;
        loop {
            if sc16is740::rx_level(bus, UART)? >= len {
                let mut buf = vec![0u8; len];
                sc16is740::receive(bus, UART, &mut buf)?;
                return Ok(buf);
            }
            let now = clock.now();
            if now >= by {
                return Err(Co2Error::Timeout(what));
            }
            self.pause(bus, clock, idle, (now + POLL).min(by))?;
        }
    }

    fn cycle(
        &mut self,
        bus: &mut dyn I2cBus,
        clock: &dyn Clock,
        idle: Idle<'_>,
        calculation: Calculation,
        pressure: u16,
    ) -> Result<Measurement, Co2Error> {
        bus.write(EXPANDER, &[REG_CONFIG, ENABLE])?;
        let on = clock.now();
        self.pause(bus, clock, idle, on + BOOT_WAIT)?;
        self.wait_rdy(bus, clock, idle, false, on + RDY_LOW_BY, "get ready")?;
        sc16is740::clear_rx(bus, UART)?;
        sc16is740::send(
            bus,
            UART,
            &lp8::write_frame(calculation, self.state.as_ref(), pressure),
        )?;
        let ack = self.receive(
            bus,
            clock,
            idle,
            lp8::WRITE_ACK.len(),
            ACK_WITHIN,
            "acknowledge",
        )?;
        lp8::check_ack(&ack)?;
        self.wait_rdy(bus, clock, idle, true, on + RDY_HIGH_BY, "finish measuring")?;
        sc16is740::clear_rx(bus, UART)?;
        sc16is740::send(bus, UART, &lp8::read_frame())?;
        let reply = self.receive(bus, clock, idle, lp8::REPLY_LEN, REPLY_WITHIN, "reply")?;
        Ok(lp8::parse_reply(&reply)?)
    }

    /// Switch the sensor and the charging off (when the app stops).
    pub fn stop(&self, bus: &mut dyn I2cBus) -> Result<(), BusError> {
        select_channel(bus, self.channel)?;
        bus.write(EXPANDER, &[REG_CONFIG, IDLE])
    }

    fn load(&mut self) {
        let Some(path) = &self.store else { return };
        let Ok(text) = std::fs::read_to_string(path) else {
            return;
        };
        let field = |name: &str| {
            text.lines()
                .find_map(|l| l.strip_prefix(name).and_then(|v| v.strip_prefix('=')))
                .map(str::trim)
        };
        let saved = field("saved_ms")
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0);
        let max_age = u64::try_from(STATE_MAX_AGE.as_millis()).unwrap_or(0);
        if unix_ms().saturating_sub(saved) > max_age {
            info!("stored LP8 state is too old; starting with an initial measurement");
            return;
        }
        let state = field("state").and_then(|hex| {
            let bytes: Option<Vec<u8>> = (0..hex.len())
                .step_by(2)
                .map(|i| {
                    hex.get(i..i + 2)
                        .and_then(|b| u8::from_str_radix(b, 16).ok())
                })
                .collect();
            bytes.and_then(|b| <[u8; lp8::STATE_LEN]>::try_from(b).ok())
        });
        if let Some(state) = state {
            self.state = Some(state);
            if let Some(due) = field("abc_due_ms").and_then(|v| v.parse().ok()) {
                self.abc_due = due;
            }
            info!("LP8 continues from its stored state");
        }
    }

    fn save(&self) {
        let Some(path) = &self.store else {
            return;
        };
        let Some(state) = &self.state else {
            // Started over after a fault: the stored state is the faulty one.
            let _ = std::fs::remove_file(path);
            return;
        };
        let hex: String = state.iter().map(|b| format!("{b:02x}")).collect();
        let text = format!(
            "state={hex}\nabc_due_ms={}\nsaved_ms={}\n",
            self.abc_due,
            unix_ms()
        );
        let tmp = path.with_extension("tmp");
        if let Err(e) = std::fs::write(&tmp, text).and_then(|()| std::fs::rename(&tmp, path)) {
            warn!("cannot store the LP8 state: {e}");
        }
    }
}
