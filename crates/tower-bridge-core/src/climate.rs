//! TOWER Climate Module behind the Bridge Module's I2C switch.
//!
//! Revisions (see twr-sdk `twr_module_climate.c`):
//! - R1: TMP112 @0x48 (temperature), SHT20 @0x40 (humidity)
//! - R2: SHT30 @0x45 (temperature and humidity)
//! - both: OPT3001 @0x44 (illuminance), MPL3115A2 @0x60 (pressure)
//!
//! A measurement cycle triggers every conversion first and then collects the
//! results as they become ready, so one cycle takes about one second (the
//! OPT3001 800 ms conversion dominates).

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tracing::{debug, info};

use crate::bus::{BusError, Clock, I2cBus};
use crate::model::{ClimateRevision, Metric, SensorSource, Values};
use crate::sensors::{SensorError, mpl3115a2, opt3001, sht20, sht30, tmp112};

/// I2C switch on the Bridge Module (one bit per downstream channel).
pub const SWITCH_ADDR: u8 = 0x70;

/// Where to look for the Climate Module.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelMode {
    /// Try channel 0, then channel 1.
    #[default]
    Auto,
    Channel0,
    Channel1,
}

impl ChannelMode {
    fn channels(self) -> &'static [u8] {
        match self {
            ChannelMode::Auto => &[0, 1],
            ChannelMode::Channel0 => &[0],
            ChannelMode::Channel1 => &[1],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SensorKind {
    Sht30,
    Sht20,
    Tmp112,
    Opt3001,
    Mpl3115a2,
}

impl SensorKind {
    pub fn name(self) -> &'static str {
        match self {
            SensorKind::Sht30 => "SHT30",
            SensorKind::Sht20 => "SHT20",
            SensorKind::Tmp112 => "TMP112",
            SensorKind::Opt3001 => "OPT3001",
            SensorKind::Mpl3115a2 => "MPL3115A2",
        }
    }
}

/// The sensor providing each value on a Climate Module revision.
pub fn sources(revision: ClimateRevision) -> Vec<SensorSource> {
    let (temperature, humidity) = match revision {
        ClimateRevision::R1 => (
            (SensorKind::Tmp112, tmp112::ADDR),
            (SensorKind::Sht20, sht20::ADDR),
        ),
        ClimateRevision::R2 => (
            (SensorKind::Sht30, sht30::ADDR),
            (SensorKind::Sht30, sht30::ADDR),
        ),
    };
    [
        (Metric::Temperature, temperature),
        (Metric::Humidity, humidity),
        (Metric::Illuminance, (SensorKind::Opt3001, opt3001::ADDR)),
        (Metric::Pressure, (SensorKind::Mpl3115a2, mpl3115a2::ADDR)),
    ]
    .into_iter()
    .map(|(metric, (kind, address))| SensorSource {
        metric,
        sensor: kind.name().into(),
        address,
    })
    .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slot {
    Absent,
    Uninit,
    Ready,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Detection {
    pub channel: u8,
    pub revision: ClimateRevision,
}

/// Result of one measurement cycle.
#[derive(Clone, Debug, Default)]
pub struct CycleReport {
    pub values: Values,
    pub errors: Vec<(SensorKind, SensorError)>,
    pub duration: Duration,
}

/// Offsets from the trigger point at which results are collected.
const FAST_FETCH_AT: Duration = Duration::from_millis(100);
const READY_POLL: Duration = Duration::from_millis(50);
const READY_POLL_COUNT: usize = 6;
const SHT20_RETRY: Duration = Duration::from_millis(20);

pub fn select_channel(bus: &mut dyn I2cBus, channel: u8) -> Result<(), BusError> {
    bus.write(SWITCH_ADDR, &[1u8 << channel])
}

pub struct ClimateModule {
    mode: ChannelMode,
    detected: Option<Detection>,
    sht30: Slot,
    sht20: Slot,
    tmp112: Slot,
    opt3001: Slot,
    mpl: Slot,
}

impl ClimateModule {
    pub fn new(mode: ChannelMode) -> Self {
        Self {
            mode,
            detected: None,
            sht30: Slot::Absent,
            sht20: Slot::Absent,
            tmp112: Slot::Absent,
            opt3001: Slot::Absent,
            mpl: Slot::Absent,
        }
    }

    pub fn detection(&self) -> Option<Detection> {
        self.detected
    }

    pub fn set_mode(&mut self, mode: ChannelMode) {
        self.mode = mode;
        self.forget();
    }

    /// Forget the detected module; the next cycle detects again.
    pub fn forget(&mut self) {
        *self = Self::new(self.mode);
    }

    /// Look for a Climate Module. `Ok(None)` when nothing answers; `Err` only
    /// for transport failures.
    pub fn detect(
        &mut self,
        bus: &mut dyn I2cBus,
        clock: &dyn Clock,
    ) -> Result<Option<Detection>, BusError> {
        self.forget();
        for &channel in self.mode.channels() {
            match select_channel(bus, channel) {
                Ok(()) => {}
                Err(e) if e.is_fatal() => return Err(e),
                Err(e) => {
                    debug!("I2C switch did not accept channel {channel}: {e}");
                    continue;
                }
            }
            // Probe with a command write: SHT3x NACKs reads when idle.
            let revision = match sht30::soft_reset(bus) {
                Ok(()) => Some(ClimateRevision::R2),
                Err(e) if e.is_fatal() => return Err(e),
                Err(_) => match sht20::soft_reset(bus) {
                    Ok(()) => Some(ClimateRevision::R1),
                    Err(e) if e.is_fatal() => return Err(e),
                    Err(_) => None,
                },
            };
            let Some(revision) = revision else {
                continue;
            };
            match revision {
                ClimateRevision::R1 => {
                    clock.sleep(sht20::RESET_TIME);
                    self.sht20 = Slot::Ready;
                    self.tmp112 = Slot::Ready;
                }
                ClimateRevision::R2 => {
                    clock.sleep(sht30::RESET_TIME);
                    self.sht30 = Slot::Ready;
                }
            }
            self.opt3001 = Slot::Uninit;
            self.mpl = Slot::Uninit;
            let detection = Detection { channel, revision };
            info!(
                "Climate Module {} found on I2C channel {channel}",
                revision.as_str()
            );
            self.detected = Some(detection);
            return Ok(Some(detection));
        }
        Ok(None)
    }

    /// Run one measurement cycle. Sensor failures yield `None` values and
    /// re-initialise that sensor next time; `Err` is returned only when the
    /// transport failed.
    pub fn measure(
        &mut self,
        bus: &mut dyn I2cBus,
        clock: &dyn Clock,
    ) -> Result<CycleReport, BusError> {
        self.measure_with(bus, clock, &mut |_, until| {
            clock.sleep_until(until);
            false
        })
    }

    /// Like [`measure`](Self::measure), but the long waits for the slow
    /// sensors go to `idle(bus, until)`, which may use the bus meanwhile (it
    /// returns whether it did, and the switch channel is then selected
    /// again). `until` is in `clock` time.
    pub fn measure_with(
        &mut self,
        bus: &mut dyn I2cBus,
        clock: &dyn Clock,
        idle: &mut dyn FnMut(&mut dyn I2cBus, Duration) -> bool,
    ) -> Result<CycleReport, BusError> {
        let started = clock.now();
        let mut report = CycleReport::default();
        let Some(detection) = self.detected else {
            return Err(BusError::Protocol("Climate Module not detected".into()));
        };
        match select_channel(bus, detection.channel) {
            Ok(()) => {}
            Err(e) if e.is_fatal() => return Err(e),
            Err(e) => {
                report.errors.push((SensorKind::Sht30, SensorError::Bus(e)));
                report.duration = clock.now() - started;
                return Ok(report);
            }
        }

        // Initialise sensors that are new or failed last time.
        let mut rec = Recorder {
            errors: &mut report.errors,
        };
        if self.sht30 == Slot::Uninit {
            let r = sht30::soft_reset(bus).map_err(SensorError::from);
            if rec.check(&mut self.sht30, SensorKind::Sht30, r)?.is_some() {
                clock.sleep(sht30::RESET_TIME);
                self.sht30 = Slot::Ready;
            }
        }
        if self.sht20 == Slot::Uninit {
            let r = sht20::soft_reset(bus).map_err(SensorError::from);
            if rec.check(&mut self.sht20, SensorKind::Sht20, r)?.is_some() {
                clock.sleep(sht20::RESET_TIME);
                self.sht20 = Slot::Ready;
            }
        }
        if self.tmp112 == Slot::Uninit {
            self.tmp112 = Slot::Ready;
        }
        if self.opt3001 == Slot::Uninit {
            let r = opt3001::init(bus).map_err(SensorError::from);
            if rec
                .check(&mut self.opt3001, SensorKind::Opt3001, r)?
                .is_some()
            {
                self.opt3001 = Slot::Ready;
            }
        }
        if self.mpl == Slot::Uninit {
            mpl3115a2::reset(bus);
            clock.sleep(mpl3115a2::RESET_TIME);
            let r = mpl3115a2::configure(bus).map_err(SensorError::from);
            if rec
                .check(&mut self.mpl, SensorKind::Mpl3115a2, r)?
                .is_some()
            {
                self.mpl = Slot::Ready;
            }
        }

        // Trigger all conversions.
        let t0 = clock.now();
        if self.sht30 == Slot::Ready {
            let r = sht30::trigger(bus).map_err(SensorError::from);
            rec.check(&mut self.sht30, SensorKind::Sht30, r)?;
        }
        if self.tmp112 == Slot::Ready {
            let r = tmp112::trigger(bus).map_err(SensorError::from);
            rec.check(&mut self.tmp112, SensorKind::Tmp112, r)?;
        }
        if self.sht20 == Slot::Ready {
            let r = sht20::trigger_humidity(bus).map_err(SensorError::from);
            rec.check(&mut self.sht20, SensorKind::Sht20, r)?;
        }
        if self.opt3001 == Slot::Ready {
            let r = opt3001::trigger(bus).map_err(SensorError::from);
            rec.check(&mut self.opt3001, SensorKind::Opt3001, r)?;
        }
        if self.mpl == Slot::Ready {
            let r = mpl3115a2::trigger(bus).map_err(SensorError::from);
            rec.check(&mut self.mpl, SensorKind::Mpl3115a2, r)?;
        }

        let values = &mut report.values;

        // Fast sensors.
        clock.sleep_until(t0 + FAST_FETCH_AT);
        if self.sht30 == Slot::Ready {
            let r = sht30::fetch(bus);
            if let Some((t, rh)) = rec.check(&mut self.sht30, SensorKind::Sht30, r)? {
                values.temperature = Some(t);
                values.humidity = Some(rh.clamp(0.0, 100.0));
            }
        }
        if self.tmp112 == Slot::Ready {
            let r = poll_ready(clock, || tmp112::is_ready(bus), 3, SHT20_RETRY)
                .and_then(|()| tmp112::fetch(bus));
            if let Some(t) = rec.check(&mut self.tmp112, SensorKind::Tmp112, r)? {
                values.temperature = Some(t);
            }
        }
        if self.sht20 == Slot::Ready {
            let mut r = sht20::fetch_humidity(bus);
            for _ in 0..3 {
                if !matches!(r, Err(SensorError::Bus(BusError::AddrNack))) {
                    break;
                }
                clock.sleep(SHT20_RETRY);
                r = sht20::fetch_humidity(bus);
            }
            if let Some(rh) = rec.check(&mut self.sht20, SensorKind::Sht20, r)? {
                values.humidity = Some(rh.clamp(0.0, 100.0));
            }
        }

        // Waiting for the slow sensors; others may use the bus meanwhile.
        let mut idle_until = |bus: &mut dyn I2cBus, until: Duration| -> Result<(), BusError> {
            if idle(bus, until) {
                match select_channel(bus, detection.channel) {
                    Err(e) if e.is_fatal() => return Err(e),
                    _ => {}
                }
            }
            Ok(())
        };

        // Barometer.
        if self.mpl == Slot::Ready {
            idle_until(bus, t0 + mpl3115a2::CONVERSION_TIME)?;
            let r = poll_ready(
                clock,
                || mpl3115a2::is_ready(bus),
                READY_POLL_COUNT,
                READY_POLL,
            )
            .and_then(|()| mpl3115a2::fetch(bus))
            .and_then(|(p, _)| {
                if (20_000.0..=120_000.0).contains(&p) {
                    Ok(p)
                } else {
                    Err(SensorError::Invalid(format!(
                        "pressure {p} Pa out of range"
                    )))
                }
            });
            if let Some(p) = rec.check(&mut self.mpl, SensorKind::Mpl3115a2, r)? {
                values.pressure = Some(p);
            }
        }

        // Light sensor.
        if self.opt3001 == Slot::Ready {
            idle_until(bus, t0 + opt3001::CONVERSION_TIME)?;
            let r = poll_ready(
                clock,
                || opt3001::is_ready(bus),
                READY_POLL_COUNT,
                READY_POLL,
            )
            .and_then(|()| opt3001::fetch(bus));
            if let Some(lux) = rec.check(&mut self.opt3001, SensorKind::Opt3001, r)? {
                values.illuminance = Some(lux);
            }
        }

        report.duration = clock.now() - started;
        Ok(report)
    }
}

/// Collects non-fatal sensor errors and marks failed sensors for re-init.
struct Recorder<'a> {
    errors: &'a mut Vec<(SensorKind, SensorError)>,
}

impl Recorder<'_> {
    fn check<T>(
        &mut self,
        slot: &mut Slot,
        kind: SensorKind,
        r: Result<T, SensorError>,
    ) -> Result<Option<T>, BusError> {
        match r {
            Ok(v) => Ok(Some(v)),
            Err(SensorError::Bus(e)) if e.is_fatal() => Err(e),
            Err(e) => {
                debug!("{} failed: {e}", kind.name());
                *slot = Slot::Uninit;
                self.errors.push((kind, e));
                Ok(None)
            }
        }
    }
}

fn poll_ready(
    clock: &dyn Clock,
    mut ready: impl FnMut() -> Result<bool, BusError>,
    attempts: usize,
    interval: Duration,
) -> Result<(), SensorError> {
    for i in 0..attempts {
        if ready()? {
            return Ok(());
        }
        if i + 1 < attempts {
            clock.sleep(interval);
        }
    }
    Err(SensorError::NotReady)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::bus::mock::{MockBus, MockClock, SharedEnv, Transaction};

    fn env() -> SharedEnv {
        Arc::new(Mutex::new(Values {
            temperature: Some(23.5),
            humidity: Some(41.0),
            illuminance: Some(512.0),
            pressure: Some(98_765.25),
        }))
    }

    fn assert_close(a: Option<f64>, b: f64, tol: f64) {
        match a {
            Some(a) => assert!((a - b).abs() <= tol, "{a} != {b}"),
            None => panic!("value missing, expected {b}"),
        }
    }

    #[test]
    fn detects_r2_and_measures() -> Result<(), BusError> {
        let mut bus = MockBus::with_climate(ClimateRevision::R2, 0, env());
        let clock = MockClock::new();
        let mut module = ClimateModule::new(ChannelMode::Auto);
        let det = module.detect(&mut bus, &clock)?;
        assert_eq!(
            det,
            Some(Detection {
                channel: 0,
                revision: ClimateRevision::R2
            })
        );
        let report = module.measure(&mut bus, &clock)?;
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        let v = report.values;
        assert_close(v.temperature, 23.5, 0.01);
        assert_close(v.humidity, 41.0, 0.01);
        assert_close(v.illuminance, 512.0, 0.1);
        assert_close(v.pressure, 98_765.25, 0.01);
        Ok(())
    }

    #[test]
    fn slow_sensor_waits_are_lent_out_and_the_channel_restored() -> Result<(), BusError> {
        let mut bus = MockBus::with_climate(ClimateRevision::R2, 1, env());
        let clock = MockClock::new();
        let mut module = ClimateModule::new(ChannelMode::Auto);
        module.detect(&mut bus, &clock)?;
        bus.log.clear();
        let mut calls = 0;
        let report = module.measure_with(&mut bus, &clock, &mut |bus, until| {
            calls += 1;
            clock.sleep_until(until);
            // Someone else uses the bus on another channel.
            let _ = select_channel(bus, 0);
            true
        })?;
        assert_eq!(calls, 2, "barometer and light sensor waits");
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert!(report.values.pressure.is_some() && report.values.illuminance.is_some());
        Ok(())
    }

    #[test]
    fn detects_r1_on_channel_1() -> Result<(), BusError> {
        let mut bus = MockBus::with_climate(ClimateRevision::R1, 1, env());
        let clock = MockClock::new();
        let mut module = ClimateModule::new(ChannelMode::Auto);
        let det = module.detect(&mut bus, &clock)?;
        assert_eq!(
            det,
            Some(Detection {
                channel: 1,
                revision: ClimateRevision::R1
            })
        );
        let report = module.measure(&mut bus, &clock)?;
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert_close(report.values.temperature, 23.5, 0.07);
        assert_close(report.values.humidity, 41.0, 0.1);
        Ok(())
    }

    #[test]
    fn sources_name_the_sensor_and_address_per_value() {
        let r1 = sources(ClimateRevision::R1);
        let t = r1.iter().find(|s| s.metric == Metric::Temperature);
        assert_eq!(
            t.map(|s| (s.sensor.as_str(), s.address)),
            Some(("TMP112", 0x48))
        );
        let r2 = sources(ClimateRevision::R2);
        assert!(r2.iter().filter(|s| s.sensor == "SHT30").count() == 2);
        assert_eq!(r2.len(), 4);
    }

    #[test]
    fn no_module() -> Result<(), BusError> {
        let mut bus = MockBus::new();
        let clock = MockClock::new();
        let mut module = ClimateModule::new(ChannelMode::Auto);
        assert_eq!(module.detect(&mut bus, &clock)?, None);
        Ok(())
    }

    #[test]
    fn steady_state_cycle_is_about_one_second() -> Result<(), BusError> {
        let mut bus = MockBus::with_climate(ClimateRevision::R2, 0, env());
        let clock = MockClock::new();
        let mut module = ClimateModule::new(ChannelMode::Channel0);
        module.detect(&mut bus, &clock)?;
        module.measure(&mut bus, &clock)?; // first cycle includes the barometer reset
        let report = module.measure(&mut bus, &clock)?;
        assert!(
            report.duration <= Duration::from_millis(1000),
            "{:?}",
            report.duration
        );
        Ok(())
    }

    #[test]
    fn steady_state_sequence_matches_drivers() -> Result<(), BusError> {
        let mut bus = MockBus::with_climate(ClimateRevision::R2, 0, env());
        let clock = MockClock::new();
        let mut module = ClimateModule::new(ChannelMode::Channel0);
        module.detect(&mut bus, &clock)?;
        module.measure(&mut bus, &clock)?;
        bus.log.clear();
        module.measure(&mut bus, &clock)?;
        assert_eq!(
            bus.log,
            vec![
                Transaction::Write(0x70, vec![0x01]),
                Transaction::Write(0x45, vec![0x24, 0x00]),
                Transaction::Write(0x44, vec![0x01, 0xCA, 0x10]),
                Transaction::Write(0x60, vec![0x26, 0x3A]),
                Transaction::Read(0x45, 6),
                Transaction::WriteRead(0x60, vec![0x00], 1),
                Transaction::WriteRead(0x60, vec![0x01], 5),
                Transaction::WriteRead(0x44, vec![0x01], 2),
                Transaction::WriteRead(0x44, vec![0x00], 2),
            ]
        );
        Ok(())
    }

    #[test]
    fn failed_sensor_is_reinitialised_next_cycle() -> Result<(), BusError> {
        let mut bus = MockBus::with_climate(ClimateRevision::R2, 0, env());
        let clock = MockClock::new();
        let mut module = ClimateModule::new(ChannelMode::Channel0);
        module.detect(&mut bus, &clock)?;
        module.measure(&mut bus, &clock)?;
        // Fail the SHT30 trigger (second transaction after the switch write).
        bus.inject = vec![];
        bus.log.clear();
        let mut faulty = FailNth {
            inner: &mut bus,
            n: 1,
        };
        let report = module.measure(&mut faulty, &clock)?;
        assert_eq!(report.values.temperature, None);
        assert_eq!(report.errors.len(), 1);
        assert!(report.values.pressure.is_some());
        bus.log.clear();
        let report = module.measure(&mut bus, &clock)?;
        assert!(report.values.temperature.is_some());
        assert!(
            bus.log
                .contains(&Transaction::Write(0x45, vec![0x30, 0xA2]))
        );
        Ok(())
    }

    #[test]
    fn fatal_error_propagates() -> Result<(), BusError> {
        let mut bus = MockBus::with_climate(ClimateRevision::R2, 0, env());
        let clock = MockClock::new();
        let mut module = ClimateModule::new(ChannelMode::Channel0);
        module.detect(&mut bus, &clock)?;
        bus.inject = vec![BusError::Hid("gone".into())];
        assert!(matches!(
            module.measure(&mut bus, &clock),
            Err(BusError::Hid(_))
        ));
        Ok(())
    }

    /// Wraps a bus and fails the n-th transaction (0-based) with a NACK.
    struct FailNth<'a> {
        inner: &'a mut MockBus,
        n: usize,
    }

    impl FailNth<'_> {
        fn tick(&mut self) -> Result<(), BusError> {
            let fail = self.n == 0;
            self.n = self.n.wrapping_sub(1);
            if fail {
                Err(BusError::AddrNack)
            } else {
                Ok(())
            }
        }
    }

    impl I2cBus for FailNth<'_> {
        fn write(&mut self, addr: u8, data: &[u8]) -> Result<(), BusError> {
            self.tick()?;
            self.inner.write(addr, data)
        }
        fn read(&mut self, addr: u8, buf: &mut [u8]) -> Result<(), BusError> {
            self.tick()?;
            self.inner.read(addr, buf)
        }
        fn write_read(&mut self, addr: u8, wr: &[u8], rd: &mut [u8]) -> Result<(), BusError> {
            self.tick()?;
            self.inner.write_read(addr, wr, rd)
        }
        fn recover(&mut self) -> Result<(), BusError> {
            Ok(())
        }
        fn info(&self) -> &crate::model::DeviceInfo {
            self.inner.info()
        }
    }
}
