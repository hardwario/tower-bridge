//! Sensirion SGPC3 TVOC sensor (TOWER VOC-LP Tag, 0x58), per Sensirion's
//! embedded-sgp driver.
//!
//! Its algorithm wants a measurement at a fixed interval: 2 s in the default
//! low-power mode, 30 s in ultra-low-power mode. twr-sdk measured every 30 s
//! without switching to ultra-low power; here the mode is set first. The
//! baseline the sensor learns is read every hour and restored after a
//! restart if it is less than a week old.

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tracing::{debug, info, warn};

use super::crc::crc8;
use crate::bus::{BusError, Clock, I2cBus};

pub const ADDR: u8 = 0x58;
/// The measurement interval in ultra-low-power mode.
pub const PERIOD: Duration = Duration::from_secs(30);

const FEATURE_SET: [u8; 2] = [0x20, 0x2F];
const SET_POWER_MODE: [u8; 2] = [0x20, 0x9F];
const INIT_PREHEAT: [u8; 2] = [0x20, 0xAE];
const MEASURE: [u8; 2] = [0x20, 0x08];
const GET_BASELINE: [u8; 2] = [0x20, 0x15];
const SET_BASELINE: [u8; 2] = [0x20, 0x1E];
const SET_HUMIDITY: [u8; 2] = [0x20, 0x61];

const COMMAND_TIME: Duration = Duration::from_millis(10);
const MEASURE_TIME: Duration = Duration::from_millis(50);
const BASELINE_EVERY: Duration = Duration::from_secs(3600);
const BASELINE_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 3600);
const GIVE_UP: u8 = 3;
/// While the sensor preheats it reads 0 ppb: not a value.
const PREHEAT: Duration = Duration::from_secs(300);

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

fn word(command: [u8; 2], value: u16) -> [u8; 5] {
    let [hi, lo] = value.to_be_bytes();
    [command[0], command[1], hi, lo, crc8(&[hi, lo], 0xFF)]
}

fn read_word(bus: &mut dyn I2cBus) -> Result<Option<u16>, BusError> {
    let mut b = [0u8; 3];
    bus.read(ADDR, &mut b)?;
    Ok((crc8(&b, 0xFF) == 0).then(|| u16::from_be_bytes([b[0], b[1]])))
}

/// Absolute humidity in g/m³ as the sensor takes it (8.8 fixed point; 0
/// switches the compensation off).
pub fn absolute_humidity(celsius: f64, rh_percent: f64) -> u16 {
    let pws = 611.2 * (17.67 * celsius / (243.5 + celsius)).exp();
    let ah = 2.165 * pws * rh_percent / 100.0 / (273.15 + celsius);
    (ah * 256.0).round().clamp(0.0, f64::from(u16::MAX)) as u16
}

/// The feature set, if an SGPC3 that can do ultra-low power (feature set 6
/// or newer) answers on the selected channel.
pub fn probe(bus: &mut dyn I2cBus) -> Result<Option<u16>, BusError> {
    let r = bus.write(ADDR, &FEATURE_SET).and_then(|()| {
        std::thread::sleep(Duration::from_millis(2));
        read_word(bus)
    });
    match r {
        // Product type 1 (SGPC3) in the top nibble, the feature set below.
        Ok(Some(w)) if w & 0xF000 == 0x1000 && w & 0x00FF >= 6 => Ok(Some(w)),
        Ok(_) => Ok(None),
        Err(e) if e.is_fatal() => Err(e),
        Err(_) => Ok(None),
    }
}

#[derive(Clone, Debug)]
pub struct Sgpc3 {
    channel: u8,
    /// Clock time of the next measurement.
    next: Duration,
    /// Clock time the baseline is next stored.
    baseline_at: Duration,
    /// Clock time of the start.
    started: Duration,
    value: Option<(f64, Duration)>,
    failures: u8,
    store: Option<PathBuf>,
}

impl Sgpc3 {
    /// Switch a sensor found on `channel` (selected) to ultra-low power and
    /// start its algorithm, from a stored baseline when there is a recent one.
    pub fn start(
        bus: &mut dyn I2cBus,
        clock: &dyn Clock,
        channel: u8,
        store: Option<PathBuf>,
    ) -> Result<Self, BusError> {
        bus.write(ADDR, &word(SET_POWER_MODE, 0))?;
        clock.sleep(COMMAND_TIME);
        bus.write(ADDR, &INIT_PREHEAT)?;
        clock.sleep(COMMAND_TIME);
        let now = clock.now();
        let sensor = Self {
            channel,
            next: now + PERIOD,
            baseline_at: now + BASELINE_EVERY,
            started: now,
            value: None,
            failures: 0,
            store,
        };
        if let Some(baseline) = sensor.stored_baseline() {
            bus.write(ADDR, &word(SET_BASELINE, baseline))?;
            clock.sleep(COMMAND_TIME);
            info!("SGPC3 continues from its stored baseline");
        }
        Ok(sensor)
    }

    pub fn channel(&self) -> u8 {
        self.channel
    }

    /// Clock time of the next measurement.
    pub fn next(&self) -> Duration {
        self.next
    }

    pub fn due(&self, now: Duration) -> bool {
        now >= self.next
    }

    pub fn gone(&self) -> bool {
        self.failures >= GIVE_UP
    }

    /// How long the sensor still preheats (reads 0 ppb), if it does.
    pub fn preheat_left(&self, now: Duration) -> Option<Duration> {
        PREHEAT
            .checked_sub(now.saturating_sub(self.started))
            .filter(|left| !left.is_zero())
    }

    /// A measurement slot that could not be served (the channel could not
    /// be selected): counts as a failure, and the grid moves on.
    pub fn missed(&mut self, now: Duration) {
        self.failures = self.failures.saturating_add(1);
        while self.next <= now {
            self.next += PERIOD;
        }
    }

    pub fn value(&self, now: Duration, max_age: Duration) -> Option<f64> {
        self.value
            .filter(|(_, at)| now.saturating_sub(*at) <= max_age)
            .map(|(v, _)| v)
    }

    /// Measure when due (the channel must be selected), with the humidity
    /// for compensation if known. Keeps to the 30 s grid.
    pub fn service(
        &mut self,
        bus: &mut dyn I2cBus,
        clock: &dyn Clock,
        climate: Option<(f64, f64)>,
    ) -> Result<Option<f64>, BusError> {
        let now = clock.now();
        if !self.due(now) {
            return Ok(None);
        }
        self.next += PERIOD;
        if self.next <= now {
            self.next = now + PERIOD;
        }
        match self.measure(bus, clock, climate) {
            Ok(ppb) => {
                self.failures = 0;
                let preheating = clock.now().saturating_sub(self.started) < PREHEAT;
                let ppb = ppb.filter(|v| !(preheating && *v == 0.0));
                if let Some(v) = ppb {
                    self.value = Some((v, clock.now()));
                }
                if clock.now() >= self.baseline_at {
                    self.baseline_at = clock.now() + BASELINE_EVERY;
                    self.save_baseline(bus, clock);
                }
                Ok(ppb)
            }
            Err(e) if e.is_fatal() => Err(e),
            Err(e) => {
                self.failures = self.failures.saturating_add(1);
                debug!("SGPC3 on channel {} failed: {e}", self.channel);
                Ok(None)
            }
        }
    }

    fn measure(
        &mut self,
        bus: &mut dyn I2cBus,
        clock: &dyn Clock,
        climate: Option<(f64, f64)>,
    ) -> Result<Option<f64>, BusError> {
        let humidity = climate.map_or(0, |(t, rh)| absolute_humidity(t, rh));
        bus.write(ADDR, &word(SET_HUMIDITY, humidity))?;
        clock.sleep(COMMAND_TIME);
        bus.write(ADDR, &MEASURE)?;
        clock.sleep(MEASURE_TIME);
        let tvoc = read_word(bus)?;
        if tvoc.is_none() {
            debug!("SGPC3 CRC mismatch");
        }
        Ok(tvoc.map(f64::from))
    }

    fn save_baseline(&self, bus: &mut dyn I2cBus, clock: &dyn Clock) {
        let Some(path) = &self.store else { return };
        let baseline = bus.write(ADDR, &GET_BASELINE).and_then(|()| {
            clock.sleep(COMMAND_TIME);
            read_word(bus)
        });
        match baseline {
            // 0: not learnt yet.
            Ok(Some(b)) if b != 0 => {
                let text = format!("baseline={b}\nsaved_ms={}\n", unix_ms());
                let tmp = path.with_extension("tmp");
                if let Err(e) =
                    std::fs::write(&tmp, text).and_then(|()| std::fs::rename(&tmp, path))
                {
                    warn!("cannot store the SGPC3 baseline: {e}");
                }
            }
            Ok(_) => {}
            Err(e) => debug!("SGPC3 baseline not read: {e}"),
        }
    }

    fn stored_baseline(&self) -> Option<u16> {
        let text = std::fs::read_to_string(self.store.as_ref()?).ok()?;
        let field = |name: &str| {
            text.lines()
                .find_map(|l| l.strip_prefix(name).and_then(|v| v.strip_prefix('=')))
                .and_then(|v| v.trim().parse::<u64>().ok())
        };
        let age = unix_ms().saturating_sub(field("saved_ms")?);
        let baseline = u16::try_from(field("baseline")?).ok()?;
        (age <= u64::try_from(BASELINE_MAX_AGE.as_millis()).unwrap_or(0) && baseline != 0)
            .then_some(baseline)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_carry_a_crc() {
        assert_eq!(word(SET_POWER_MODE, 0), [0x20, 0x9F, 0x00, 0x00, 0x81]);
        // The feature set read from the real tag: 10 06 49.
        assert_eq!(crc8(&[0x10, 0x06], 0xFF), 0x49);
    }

    #[test]
    fn absolute_humidity_in_fixed_point() {
        // 25 °C, 50 %: about 11.5 g/m³.
        let ah = f64::from(absolute_humidity(25.0, 50.0)) / 256.0;
        assert!((ah - 11.5).abs() < 0.1, "{ah}");
        assert_eq!(absolute_humidity(20.0, 0.0), 0);
    }
}
