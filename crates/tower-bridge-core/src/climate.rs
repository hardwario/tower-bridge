//! Sensors behind the Bridge Module's I2C switch: the TOWER Climate Module
//! and the TOWER sensor tags, on either switch channel.
//!
//! Parts and addresses (see twr-sdk):
//! - SHT30 @0x45: temperature and humidity (Climate Module R2)
//! - SHT20 @0x40: humidity, and temperature when there is no TMP112
//!   (Climate Module R1, Humidity Tag)
//! - TMP112 @0x48: temperature (Climate Module R1, Temperature Tag; the tag
//!   can also sit at 0x49)
//! - OPT3001 @0x44: illuminance (Climate Module, Lux Meter Tag; the tag can
//!   also sit at 0x45)
//! - MPL3115A2 @0x60: pressure (Climate Module, Barometer Tag)
//!
//! Every channel is probed part by part, at the default address first. A
//! Climate Module is recognised by its parts at their default addresses (an
//! SHT30, or an SHT20 with a TMP112); any other part is named after the tag
//! that carries it. One part of each kind is used per channel.
//!
//! A measurement cycle triggers every conversion on a channel first and then
//! collects the results as they become ready, so one channel takes about one
//! second (the OPT3001 800 ms conversion dominates). Channels are measured
//! one after the other, because the same addresses are used on both.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tracing::{debug, info};

use crate::bus::{BusError, Clock, I2cBus};
use crate::model::{Metric, ModuleKind, SensorSource, Values};
use crate::sensors::{SensorError, mpl3115a2, opt3001, sht20, sht30, tmp112};

/// I2C switch on the Bridge Module (one bit per downstream channel).
pub const SWITCH_ADDR: u8 = 0x70;

/// Which switch channels to look for sensors on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelMode {
    /// Both channels.
    #[default]
    Auto,
    Channel0,
    Channel1,
}

impl ChannelMode {
    pub fn channels(self) -> &'static [u8] {
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

    /// The address on a Climate Module (and a tag's default).
    pub fn default_address(self) -> u8 {
        match self {
            SensorKind::Sht30 => sht30::ADDR,
            SensorKind::Sht20 => sht20::ADDR,
            SensorKind::Tmp112 => tmp112::ADDR,
            SensorKind::Opt3001 => opt3001::ADDR,
            SensorKind::Mpl3115a2 => mpl3115a2::ADDR,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slot {
    Absent,
    Uninit,
    Ready,
}

/// One sensor part on a channel.
#[derive(Clone, Copy, Debug)]
struct Part {
    slot: Slot,
    /// Failed measurements in a row.
    fails: u8,
    /// 7-bit I2C address it answered at.
    addr: u8,
}

impl Part {
    const ABSENT: Part = Part {
        slot: Slot::Absent,
        fails: 0,
        addr: 0,
    };

    fn present(self) -> bool {
        self.slot != Slot::Absent
    }

    fn ready(self) -> bool {
        self.slot == Slot::Ready
    }

    fn found(slot: Slot, addr: u8) -> Part {
        Part {
            slot,
            fails: 0,
            addr,
        }
    }
}

/// A part that fails this many measurements in a row is taken as unplugged;
/// the periodic probe finds it again when it comes back.
const PART_GIVE_UP: u8 = 3;

/// Result of measuring one channel.
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

/// `Ok(Some(_))` when the part answered, `Ok(None)` when it did not;
/// transport failures propagate.
fn answered<T>(r: Result<T, BusError>) -> Result<Option<T>, BusError> {
    match r {
        Ok(v) => Ok(Some(v)),
        Err(e) if e.is_fatal() => Err(e),
        Err(_) => Ok(None),
    }
}

/// The parts found on one switch channel.
#[derive(Clone, Debug)]
pub struct ChannelSensors {
    channel: u8,
    sht30: Part,
    sht20: Part,
    tmp112: Part,
    opt3001: Part,
    mpl: Part,
}

impl ChannelSensors {
    fn new(channel: u8) -> Self {
        Self {
            channel,
            sht30: Part::ABSENT,
            sht20: Part::ABSENT,
            tmp112: Part::ABSENT,
            opt3001: Part::ABSENT,
            mpl: Part::ABSENT,
        }
    }

    pub fn channel(&self) -> u8 {
        self.channel
    }

    pub fn is_empty(&self) -> bool {
        !(self.sht30.present()
            || self.sht20.present()
            || self.tmp112.present()
            || self.opt3001.present()
            || self.mpl.present())
    }

    /// Look for the parts not present yet (the channel must be selected).
    /// Returns whether one was found.
    fn probe(&mut self, bus: &mut dyn I2cBus, clock: &dyn Clock) -> Result<bool, BusError> {
        let mut found = false;
        // The light sensor first, by its ID: at 0x45 an OPT3001 would also
        // acknowledge the SHT30 soft reset below. 0x45 is not probed while an
        // SHT30 holds it.
        if !self.opt3001.present() {
            for addr in [opt3001::ADDR, opt3001::ALT_ADDR] {
                if addr == sht30::ADDR && self.sht30.present() {
                    continue;
                }
                if answered(opt3001::device_id(bus, addr))? == Some(opt3001::DEVICE_ID) {
                    self.opt3001 = Part::found(Slot::Uninit, addr);
                    found = true;
                    break;
                }
            }
        }
        // SHT humidity sensors only acknowledge commands, so probe with a
        // soft reset (an idle SHT3x does not even acknowledge a read).
        let lux_at_sht30 = self.opt3001.present() && self.opt3001.addr == sht30::ADDR;
        if !self.sht30.present() && !lux_at_sht30 && answered(sht30::soft_reset(bus))?.is_some() {
            clock.sleep(sht30::RESET_TIME);
            self.sht30 = Part::found(Slot::Ready, sht30::ADDR);
            found = true;
        }
        if !self.sht20.present() && answered(sht20::soft_reset(bus))?.is_some() {
            clock.sleep(sht20::RESET_TIME);
            self.sht20 = Part::found(Slot::Ready, sht20::ADDR);
            found = true;
        }
        if !self.tmp112.present() {
            for addr in [tmp112::ADDR, tmp112::ALT_ADDR] {
                if answered(tmp112::probe(bus, addr))?.is_some() {
                    self.tmp112 = Part::found(Slot::Ready, addr);
                    found = true;
                    break;
                }
            }
        }
        if !self.mpl.present() && answered(mpl3115a2::who_am_i(bus))? == Some(mpl3115a2::WHO_AM_I) {
            self.mpl = Part::found(Slot::Uninit, mpl3115a2::ADDR);
            found = true;
        }
        Ok(found)
    }

    fn part_of(&self, kind: SensorKind) -> &Part {
        match kind {
            SensorKind::Sht30 => &self.sht30,
            SensorKind::Sht20 => &self.sht20,
            SensorKind::Tmp112 => &self.tmp112,
            SensorKind::Opt3001 => &self.opt3001,
            SensorKind::Mpl3115a2 => &self.mpl,
        }
    }

    /// Whether `kind` is present at its Climate Module address.
    fn at_default(&self, kind: SensorKind) -> bool {
        let part = self.part_of(kind);
        part.present() && part.addr == kind.default_address()
    }

    /// The Climate Module whose parts are on this channel, if any.
    fn climate_module(&self) -> Option<ModuleKind> {
        let lux_or_baro = self.at_default(SensorKind::Opt3001) || self.mpl.present();
        if self.at_default(SensorKind::Sht30) && lux_or_baro {
            Some(ModuleKind::ClimateModuleR2)
        } else if self.at_default(SensorKind::Sht20) && self.at_default(SensorKind::Tmp112) {
            Some(ModuleKind::ClimateModuleR1)
        } else {
            None
        }
    }

    fn module_of(&self, kind: SensorKind) -> ModuleKind {
        let tag = match kind {
            SensorKind::Sht30 | SensorKind::Sht20 => ModuleKind::HumidityTag,
            SensorKind::Tmp112 => ModuleKind::TemperatureTag,
            SensorKind::Opt3001 => ModuleKind::LuxMeterTag,
            SensorKind::Mpl3115a2 => ModuleKind::BarometerTag,
        };
        // A part at a tag's alternative address is never the Climate Module's.
        match self.climate_module() {
            Some(module) if self.at_default(kind) => module,
            _ => tag,
        }
    }

    /// The part measuring temperature: SHT30, else TMP112, else SHT20.
    fn temperature_part(&self) -> Option<SensorKind> {
        if self.sht30.present() {
            Some(SensorKind::Sht30)
        } else if self.tmp112.present() {
            Some(SensorKind::Tmp112)
        } else if self.sht20.present() {
            Some(SensorKind::Sht20)
        } else {
            None
        }
    }

    fn humidity_part(&self) -> Option<SensorKind> {
        if self.sht30.present() {
            Some(SensorKind::Sht30)
        } else if self.sht20.present() {
            Some(SensorKind::Sht20)
        } else {
            None
        }
    }

    pub fn sources(&self) -> Vec<SensorSource> {
        [
            (Metric::Temperature, self.temperature_part()),
            (Metric::Humidity, self.humidity_part()),
            (
                Metric::Illuminance,
                self.opt3001.present().then_some(SensorKind::Opt3001),
            ),
            (
                Metric::Pressure,
                self.mpl.present().then_some(SensorKind::Mpl3115a2),
            ),
        ]
        .into_iter()
        .filter_map(|(metric, kind)| {
            kind.map(|kind| SensorSource {
                metric,
                channel: self.channel,
                module: self.module_of(kind),
                sensor: kind.name().into(),
                address: self.part_of(kind).addr,
            })
        })
        .collect()
    }

    fn part(&mut self, kind: SensorKind) -> &mut Part {
        match kind {
            SensorKind::Sht30 => &mut self.sht30,
            SensorKind::Sht20 => &mut self.sht20,
            SensorKind::Tmp112 => &mut self.tmp112,
            SensorKind::Opt3001 => &mut self.opt3001,
            SensorKind::Mpl3115a2 => &mut self.mpl,
        }
    }

    /// A measurement of `kind` succeeded: its failures in a row start over.
    fn measured(&mut self, kind: SensorKind) {
        self.part(kind).fails = 0;
    }

    /// Sort out a part's result: failures are recorded and the part is set
    /// up again next time, or given up after [`PART_GIVE_UP`] failed
    /// measurements in a row (a set-up that works does not reset the count;
    /// see [`measured`](Self::measured)). `Err` only for transport failures.
    fn check<T>(
        &mut self,
        errors: &mut Vec<(SensorKind, SensorError)>,
        kind: SensorKind,
        r: Result<T, SensorError>,
    ) -> Result<Option<T>, BusError> {
        let channel = self.channel;
        let part = self.part(kind);
        match r {
            Ok(v) => Ok(Some(v)),
            Err(SensorError::Bus(e)) if e.is_fatal() => Err(e),
            Err(e) => {
                debug!("{} on channel {channel} failed: {e}", kind.name());
                part.fails = part.fails.saturating_add(1);
                if part.fails >= PART_GIVE_UP {
                    info!("{} on channel {channel} stopped answering", kind.name());
                    *part = Part::ABSENT;
                } else {
                    part.slot = Slot::Uninit;
                }
                errors.push((kind, e));
                Ok(None)
            }
        }
    }

    /// Measure this channel; see [`Sensors::measure_with`].
    fn measure_with(
        &mut self,
        bus: &mut dyn I2cBus,
        clock: &dyn Clock,
        idle: &mut dyn FnMut(&mut dyn I2cBus, Duration) -> bool,
    ) -> Result<CycleReport, BusError> {
        let started = clock.now();
        let mut report = CycleReport::default();
        let channel = self.channel;
        match select_channel(bus, channel) {
            Ok(()) => {}
            Err(e) if e.is_fatal() => return Err(e),
            Err(e) => {
                debug!("I2C switch did not accept channel {channel}: {e}");
                report.duration = clock.now() - started;
                return Ok(report);
            }
        }
        let errors = &mut report.errors;
        let temperature = self.temperature_part();
        let humidity = self.humidity_part();
        let use_sht20 =
            temperature == Some(SensorKind::Sht20) || humidity == Some(SensorKind::Sht20);
        let use_tmp112 = temperature == Some(SensorKind::Tmp112);
        let tmp112_addr = self.tmp112.addr;
        let opt3001_addr = self.opt3001.addr;

        // Long waits go to the idle hook, which may use the bus meanwhile (a
        // relay on the other channel); then this channel is selected again.
        // Returns false when that failed: reading now would reach the other
        // channel's sensors at the same addresses.
        let mut idle_until = |bus: &mut dyn I2cBus, until: Duration| -> Result<bool, BusError> {
            if !idle(bus, until) {
                return Ok(true);
            }
            match select_channel(bus, channel) {
                Ok(()) => Ok(true),
                Err(e) if e.is_fatal() => Err(e),
                Err(e) => {
                    debug!("cannot select I2C channel {channel} again: {e}");
                    Ok(false)
                }
            }
        };
        let mut on_channel = true;

        // Set up parts that are new or failed last time.
        if self.sht30.slot == Slot::Uninit {
            let r = sht30::soft_reset(bus).map_err(SensorError::from);
            if self.check(errors, SensorKind::Sht30, r)?.is_some() {
                clock.sleep(sht30::RESET_TIME);
                self.sht30.slot = Slot::Ready;
            }
        }
        if use_sht20 && self.sht20.slot == Slot::Uninit {
            let r = sht20::soft_reset(bus).map_err(SensorError::from);
            if self.check(errors, SensorKind::Sht20, r)?.is_some() {
                clock.sleep(sht20::RESET_TIME);
                self.sht20.slot = Slot::Ready;
            }
        }
        if self.tmp112.slot == Slot::Uninit {
            self.tmp112.slot = Slot::Ready;
        }
        if self.opt3001.slot == Slot::Uninit {
            let r = opt3001::init(bus, opt3001_addr).map_err(SensorError::from);
            if self.check(errors, SensorKind::Opt3001, r)?.is_some() {
                self.opt3001.slot = Slot::Ready;
            }
        }
        if self.mpl.slot == Slot::Uninit {
            mpl3115a2::reset(bus);
            // A second of reset: keep the relay responsive meanwhile.
            on_channel = idle_until(bus, clock.now() + mpl3115a2::RESET_TIME)?;
            if on_channel {
                let r = mpl3115a2::configure(bus).map_err(SensorError::from);
                if self.check(errors, SensorKind::Mpl3115a2, r)?.is_some() {
                    self.mpl.slot = Slot::Ready;
                }
            }
        }
        if !on_channel {
            report.duration = clock.now() - started;
            return Ok(report);
        }

        // Trigger all conversions.
        let t0 = clock.now();
        if self.sht30.ready() {
            let r = sht30::trigger(bus).map_err(SensorError::from);
            self.check(errors, SensorKind::Sht30, r)?;
        }
        if use_tmp112 && self.tmp112.ready() {
            let r = tmp112::trigger(bus, tmp112_addr).map_err(SensorError::from);
            self.check(errors, SensorKind::Tmp112, r)?;
        }
        // The SHT20 converts one quantity at a time: temperature first when
        // it is the thermometer, humidity right after.
        let sht20_temperature = temperature == Some(SensorKind::Sht20);
        if use_sht20 && self.sht20.ready() {
            let r = if sht20_temperature {
                sht20::trigger_temperature(bus)
            } else {
                sht20::trigger_humidity(bus)
            }
            .map_err(SensorError::from);
            self.check(errors, SensorKind::Sht20, r)?;
        }
        if self.opt3001.ready() {
            let r = opt3001::trigger(bus, opt3001_addr).map_err(SensorError::from);
            self.check(errors, SensorKind::Opt3001, r)?;
        }
        if self.mpl.ready() {
            let r = mpl3115a2::trigger(bus).map_err(SensorError::from);
            self.check(errors, SensorKind::Mpl3115a2, r)?;
        }

        let mut values = Values::default();

        // Fast sensors.
        clock.sleep_until(t0 + FAST_FETCH_AT);
        if self.sht30.ready() {
            let r = sht30::fetch(bus);
            if let Some((t, rh)) = self.check(errors, SensorKind::Sht30, r)? {
                self.measured(SensorKind::Sht30);
                values.temperature = Some(t);
                values.humidity = Some(rh.clamp(0.0, 100.0));
            }
        }
        if use_tmp112 && self.tmp112.ready() {
            let r = poll_ready(clock, || tmp112::is_ready(bus, tmp112_addr), 3, SHT20_RETRY)
                .and_then(|()| tmp112::fetch(bus, tmp112_addr));
            if let Some(t) = self.check(errors, SensorKind::Tmp112, r)? {
                self.measured(SensorKind::Tmp112);
                values.temperature = Some(t);
            }
        }
        if use_sht20 && self.sht20.ready() && sht20_temperature {
            let r = sht20_fetch(clock, || sht20::fetch_temperature(bus));
            if let Some(t) = self.check(errors, SensorKind::Sht20, r)? {
                self.measured(SensorKind::Sht20);
                values.temperature = Some(t);
            }
            if humidity == Some(SensorKind::Sht20) && self.sht20.ready() {
                let r = sht20::trigger_humidity(bus).map_err(SensorError::from);
                if self.check(errors, SensorKind::Sht20, r)?.is_some() {
                    clock.sleep(sht20::HUMIDITY_TIME);
                }
            }
        }
        if humidity == Some(SensorKind::Sht20) && self.sht20.ready() {
            let r = sht20_fetch(clock, || sht20::fetch_humidity(bus));
            if let Some(rh) = self.check(errors, SensorKind::Sht20, r)? {
                self.measured(SensorKind::Sht20);
                values.humidity = Some(rh.clamp(0.0, 100.0));
            }
        }

        // Barometer (waiting for the slow sensors goes to the idle hook).
        if self.mpl.ready() {
            on_channel = idle_until(bus, t0 + mpl3115a2::CONVERSION_TIME)?;
        }
        if self.mpl.ready() && on_channel {
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
            if let Some(p) = self.check(errors, SensorKind::Mpl3115a2, r)? {
                self.measured(SensorKind::Mpl3115a2);
                values.pressure = Some(p);
            }
        }

        // Light sensor.
        if self.opt3001.ready() && on_channel {
            on_channel = idle_until(bus, t0 + opt3001::CONVERSION_TIME)?;
        }
        if self.opt3001.ready() && on_channel {
            let r = poll_ready(
                clock,
                || opt3001::is_ready(bus, opt3001_addr),
                READY_POLL_COUNT,
                READY_POLL,
            )
            .and_then(|()| opt3001::fetch(bus, opt3001_addr));
            if let Some(lux) = self.check(errors, SensorKind::Opt3001, r)? {
                self.measured(SensorKind::Opt3001);
                values.illuminance = Some(lux);
            }
        }

        report.values = values;
        report.duration = clock.now() - started;
        Ok(report)
    }
}

/// An SHT20 in no-hold mode does not acknowledge a read while it converts;
/// try a few times.
fn sht20_fetch(
    clock: &dyn Clock,
    mut fetch: impl FnMut() -> Result<f64, SensorError>,
) -> Result<f64, SensorError> {
    let mut r = fetch();
    for _ in 0..3 {
        if !matches!(r, Err(SensorError::Bus(BusError::AddrNack))) {
            break;
        }
        clock.sleep(SHT20_RETRY);
        r = fetch();
    }
    r
}

/// The sensors on all switch channels.
pub struct Sensors {
    mode: ChannelMode,
    /// Channels with at least one part, in channel order.
    channels: Vec<ChannelSensors>,
}

impl Sensors {
    pub fn new(mode: ChannelMode) -> Self {
        Self {
            mode,
            channels: Vec::new(),
        }
    }

    pub fn set_mode(&mut self, mode: ChannelMode) {
        self.mode = mode;
        self.forget();
    }

    /// Forget every part found; the next [`detect`](Self::detect) starts over.
    pub fn forget(&mut self) {
        self.channels.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.channels.is_empty()
    }

    /// Forget the parts on one channel, e.g. after it stopped answering.
    pub fn drop_channel(&mut self, channel: u8) {
        self.channels.retain(|c| c.channel != channel);
    }

    /// Which sensor provides each value, ordered by channel and metric.
    pub fn sources(&self) -> Vec<SensorSource> {
        self.channels.iter().flat_map(|c| c.sources()).collect()
    }

    /// Look for parts not found yet on every channel of the mode; already
    /// found parts are left alone, so this can run between measurements.
    /// Returns whether a part was found. `Err` only for transport failures.
    pub fn detect(&mut self, bus: &mut dyn I2cBus, clock: &dyn Clock) -> Result<bool, BusError> {
        let mut found = false;
        for &channel in self.mode.channels() {
            match select_channel(bus, channel) {
                Ok(()) => {}
                Err(e) if e.is_fatal() => return Err(e),
                Err(e) => {
                    debug!("I2C switch did not accept channel {channel}: {e}");
                    continue;
                }
            }
            let index = self.channels.iter().position(|c| c.channel == channel);
            let mut sensors = match index {
                Some(i) => self.channels[i].clone(),
                None => ChannelSensors::new(channel),
            };
            if !sensors.probe(bus, clock)? {
                continue;
            }
            found = true;
            let mut names: Vec<String> = sensors
                .sources()
                .iter()
                .map(|s| format!("{} {}", s.module.name(), s.sensor))
                .collect();
            names.dedup();
            info!("I2C channel {channel}: {}", names.join(", "));
            match index {
                Some(i) => self.channels[i] = sensors,
                None => {
                    self.channels.push(sensors);
                    self.channels.sort_by_key(|c| c.channel);
                }
            }
        }
        Ok(found)
    }

    /// Run one measurement cycle on every channel. Sensor failures yield
    /// `None` values; `Err` is returned only when the transport failed.
    pub fn measure(
        &mut self,
        bus: &mut dyn I2cBus,
        clock: &dyn Clock,
    ) -> Result<Vec<(u8, CycleReport)>, BusError> {
        self.measure_with(bus, clock, &mut |_, until| {
            clock.sleep_until(until);
            false
        })
    }

    /// Like [`measure`](Self::measure), but the long waits for the slow
    /// sensors go to `idle(bus, until)`, which may use the bus meanwhile (it
    /// returns whether it did, and the switch channel is then selected
    /// again). `until` is in `clock` time. Parts that keep failing are
    /// dropped, and so are channels left without parts.
    pub fn measure_with(
        &mut self,
        bus: &mut dyn I2cBus,
        clock: &dyn Clock,
        idle: &mut dyn FnMut(&mut dyn I2cBus, Duration) -> bool,
    ) -> Result<Vec<(u8, CycleReport)>, BusError> {
        let mut reports = Vec::with_capacity(self.channels.len());
        for sensors in &mut self.channels {
            let report = sensors.measure_with(bus, clock, idle)?;
            reports.push((sensors.channel, report));
        }
        self.channels.retain(|c| !c.is_empty());
        Ok(reports)
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
    use crate::bus::mock::{
        MockBus, MockClock, SharedEnv, SimMpl3115a2, SimOpt3001, SimSht20, SimTmp112, Transaction,
    };
    use crate::model::ClimateRevision;

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

    fn modules(sensors: &Sensors) -> Vec<(u8, ModuleKind)> {
        crate::model::modules(&sensors.sources())
    }

    /// The only channel's report.
    fn single(reports: Vec<(u8, CycleReport)>) -> CycleReport {
        assert_eq!(reports.len(), 1, "{reports:?}");
        reports
            .into_iter()
            .next()
            .map(|(_, r)| r)
            .unwrap_or_default()
    }

    /// Humidity, Lux Meter and Barometer Tags.
    fn attach_tags(bus: &mut MockBus, channel: usize, env: SharedEnv) {
        bus.attach(channel, 0x40, Box::new(SimSht20::new(env.clone())));
        bus.attach(channel, 0x44, Box::new(SimOpt3001::new(env.clone())));
        bus.attach(channel, 0x60, Box::new(SimMpl3115a2::new(env)));
    }

    #[test]
    fn detects_r2_and_measures() -> Result<(), BusError> {
        let mut bus = MockBus::with_climate(ClimateRevision::R2, 0, env());
        let clock = MockClock::new();
        let mut sensors = Sensors::new(ChannelMode::Auto);
        assert!(sensors.detect(&mut bus, &clock)?);
        assert_eq!(modules(&sensors), vec![(0, ModuleKind::ClimateModuleR2)]);
        let report = single(sensors.measure(&mut bus, &clock)?);
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
        let mut sensors = Sensors::new(ChannelMode::Auto);
        sensors.detect(&mut bus, &clock)?;
        bus.log.clear();
        let lent = std::cell::RefCell::new(Vec::new());
        let mut idle = |bus: &mut dyn I2cBus, until: Duration| {
            lent.borrow_mut().push(until);
            clock.sleep_until(until);
            // Someone else uses the bus on another channel.
            let _ = select_channel(bus, 0);
            true
        };
        let report = single(sensors.measure_with(&mut bus, &clock, &mut idle)?);
        assert_eq!(
            lent.borrow().len(),
            3,
            "the barometer's reset, then its conversion and the light sensor's"
        );
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert!(report.values.pressure.is_some() && report.values.illuminance.is_some());
        lent.borrow_mut().clear();
        single(sensors.measure_with(&mut bus, &clock, &mut idle)?);
        assert_eq!(lent.borrow().len(), 2, "no reset in later cycles");
        Ok(())
    }

    /// Fails the switch write that follows an idle hook which used the bus.
    struct SwitchFailsAfterIdle<'a> {
        inner: &'a mut MockBus,
        fail: Arc<std::sync::atomic::AtomicBool>,
    }

    impl I2cBus for SwitchFailsAfterIdle<'_> {
        fn write(&mut self, addr: u8, data: &[u8]) -> Result<(), BusError> {
            if addr == SWITCH_ADDR && self.fail.swap(false, std::sync::atomic::Ordering::Relaxed) {
                return Err(BusError::AddrNack);
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
            Ok(())
        }
        fn info(&self) -> &crate::model::DeviceInfo {
            self.inner.info()
        }
    }

    #[test]
    fn a_failed_reselect_after_idle_does_not_read_the_other_channel() -> Result<(), BusError> {
        let other: SharedEnv = Arc::new(Mutex::new(Values {
            illuminance: Some(9999.0),
            pressure: Some(90_000.0),
            ..Values::default()
        }));
        let mut bus = MockBus::with_climate(ClimateRevision::R2, 0, env());
        bus.attach(1, 0x44, Box::new(SimOpt3001::new(other.clone())));
        bus.attach(1, 0x60, Box::new(SimMpl3115a2::new(other)));
        let clock = MockClock::new();
        let mut sensors = Sensors::new(ChannelMode::Channel0);
        sensors.detect(&mut bus, &clock)?;
        sensors.measure(&mut bus, &clock)?;
        let fail = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut faulty = SwitchFailsAfterIdle {
            inner: &mut bus,
            fail: fail.clone(),
        };
        let report = single(
            sensors.measure_with(&mut faulty, &clock, &mut |bus, until| {
                clock.sleep_until(until);
                let _ = select_channel(bus, 1);
                fail.store(true, std::sync::atomic::Ordering::Relaxed);
                true
            })?,
        );
        assert_eq!(
            report.values.pressure, None,
            "not the other channel's 900 hPa"
        );
        assert_eq!(report.values.illuminance, None);
        assert!(
            report.values.temperature.is_some(),
            "read before the idle wait"
        );
        assert_eq!(sensors.sources().len(), 4, "no part blamed for the switch");
        Ok(())
    }

    /// Acknowledges every command but never returns a measurement.
    struct Mute;

    impl crate::bus::mock::SimDevice for Mute {
        fn write(&mut self, _data: &[u8]) -> Result<(), BusError> {
            Ok(())
        }
        fn read(&mut self, _buf: &mut [u8]) -> Result<(), BusError> {
            Err(BusError::DataNack)
        }
    }

    #[test]
    fn a_part_that_never_measures_is_dropped() -> Result<(), BusError> {
        let mut bus = MockBus::new();
        bus.attach(0, 0x45, Box::new(Mute));
        let clock = MockClock::new();
        let mut sensors = Sensors::new(ChannelMode::Channel0);
        sensors.detect(&mut bus, &clock)?;
        assert_eq!(sensors.sources().len(), 2, "taken for an SHT30");
        for _ in 1..PART_GIVE_UP {
            sensors.measure(&mut bus, &clock)?;
        }
        assert!(!sensors.is_empty(), "still trying");
        sensors.measure(&mut bus, &clock)?;
        assert!(
            sensors.is_empty(),
            "dropped although its set-up always works"
        );
        Ok(())
    }

    #[test]
    fn detects_r1_on_channel_1() -> Result<(), BusError> {
        let mut bus = MockBus::with_climate(ClimateRevision::R1, 1, env());
        let clock = MockClock::new();
        let mut sensors = Sensors::new(ChannelMode::Auto);
        sensors.detect(&mut bus, &clock)?;
        assert_eq!(modules(&sensors), vec![(1, ModuleKind::ClimateModuleR1)]);
        let report = single(sensors.measure(&mut bus, &clock)?);
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert_close(report.values.temperature, 23.5, 0.07);
        assert_close(report.values.humidity, 41.0, 0.1);
        Ok(())
    }

    #[test]
    fn the_same_tag_on_both_channels() -> Result<(), BusError> {
        let tag = |t: f64, rh: f64| -> SharedEnv {
            Arc::new(Mutex::new(Values {
                temperature: Some(t),
                humidity: Some(rh),
                ..Values::default()
            }))
        };
        // The I2C switch keeps the two SHT20 at 0x40 apart.
        let mut bus = MockBus::new();
        bus.attach(0, 0x40, Box::new(SimSht20::new(tag(21.0, 40.0))));
        bus.attach(1, 0x40, Box::new(SimSht20::new(tag(8.0, 85.0))));
        let clock = MockClock::new();
        let mut sensors = Sensors::new(ChannelMode::Auto);
        sensors.detect(&mut bus, &clock)?;
        assert_eq!(
            modules(&sensors),
            vec![(0, ModuleKind::HumidityTag), (1, ModuleKind::HumidityTag)]
        );
        let sources = sensors.sources();
        assert_eq!(
            crate::model::source_label(&sources, Metric::Humidity, 1).as_deref(),
            Some("Humidity Tag, channel 1"),
            "the same name on both channels: the channel tells them apart"
        );
        let reports = sensors.measure(&mut bus, &clock)?;
        let (_, indoor) = &reports[0];
        let (_, outdoor) = &reports[1];
        assert_close(indoor.values.humidity, 40.0, 0.1);
        assert_close(outdoor.values.humidity, 85.0, 0.1);
        assert_close(outdoor.values.temperature, 8.0, 0.05);
        Ok(())
    }

    #[test]
    fn climate_module_and_tags_on_two_channels() -> Result<(), BusError> {
        let outdoor: SharedEnv = Arc::new(Mutex::new(Values {
            temperature: Some(12.25),
            humidity: Some(70.0),
            illuminance: Some(9000.0),
            pressure: Some(98_000.0),
        }));
        let mut bus = MockBus::with_climate(ClimateRevision::R1, 0, env());
        attach_tags(&mut bus, 1, outdoor);
        let clock = MockClock::new();
        let mut sensors = Sensors::new(ChannelMode::Auto);
        sensors.detect(&mut bus, &clock)?;
        assert_eq!(
            modules(&sensors),
            vec![
                (0, ModuleKind::ClimateModuleR1),
                (1, ModuleKind::HumidityTag),
                (1, ModuleKind::LuxMeterTag),
                (1, ModuleKind::BarometerTag),
            ]
        );
        let tag_temperature = sensors
            .sources()
            .into_iter()
            .find(|s| s.channel == 1 && s.metric == Metric::Temperature);
        assert_eq!(
            tag_temperature.map(|s| (s.module, s.sensor, s.address)),
            Some((ModuleKind::HumidityTag, "SHT20".to_string(), 0x40))
        );
        let reports = sensors.measure(&mut bus, &clock)?;
        assert_eq!(reports.len(), 2);
        let (ch0, indoor) = &reports[0];
        let (ch1, tags) = &reports[1];
        assert_eq!((*ch0, *ch1), (0, 1));
        assert!(indoor.errors.is_empty() && tags.errors.is_empty());
        assert_close(indoor.values.temperature, 23.5, 0.07);
        assert_close(tags.values.temperature, 12.25, 0.05);
        assert_close(tags.values.humidity, 70.0, 0.1);
        assert_close(tags.values.illuminance, 9000.0, 10.0);
        assert_close(tags.values.pressure, 98_000.0, 0.5);
        Ok(())
    }

    #[test]
    fn a_tag_plugged_in_later_is_found_by_the_next_detect() -> Result<(), BusError> {
        let e = env();
        let mut bus = MockBus::with_climate(ClimateRevision::R2, 0, e.clone());
        let clock = MockClock::new();
        let mut sensors = Sensors::new(ChannelMode::Auto);
        sensors.detect(&mut bus, &clock)?;
        assert!(!sensors.detect(&mut bus, &clock)?, "nothing new");
        bus.attach(1, 0x44, Box::new(SimOpt3001::new(e)));
        assert!(sensors.detect(&mut bus, &clock)?);
        assert_eq!(
            modules(&sensors),
            vec![
                (0, ModuleKind::ClimateModuleR2),
                (1, ModuleKind::LuxMeterTag)
            ]
        );
        Ok(())
    }

    #[test]
    fn an_unplugged_tag_is_dropped_after_repeated_failures() -> Result<(), BusError> {
        let mut bus = MockBus::with_climate(ClimateRevision::R2, 0, env());
        attach_tags(&mut bus, 1, env());
        let clock = MockClock::new();
        let mut sensors = Sensors::new(ChannelMode::Auto);
        sensors.detect(&mut bus, &clock)?;
        sensors.measure(&mut bus, &clock)?;
        bus.detach_all(1);
        for _ in 0..PART_GIVE_UP {
            let reports = sensors.measure(&mut bus, &clock)?;
            assert!(reports.iter().any(|(c, r)| *c == 0 && !r.values.is_empty()));
        }
        assert_eq!(modules(&sensors), vec![(0, ModuleKind::ClimateModuleR2)]);
        Ok(())
    }

    #[test]
    fn a_lux_meter_tag_at_0x45_is_not_taken_for_an_sht30() -> Result<(), BusError> {
        let mut bus = MockBus::new();
        bus.attach(1, 0x45, Box::new(SimOpt3001::new(env())));
        let clock = MockClock::new();
        let mut sensors = Sensors::new(ChannelMode::Auto);
        sensors.detect(&mut bus, &clock)?;
        let sources = sensors.sources();
        assert_eq!(sources.len(), 1, "{sources:?}");
        assert_eq!(
            (sources[0].metric, sources[0].module, sources[0].address),
            (Metric::Illuminance, ModuleKind::LuxMeterTag, 0x45)
        );
        let report = single(sensors.measure(&mut bus, &clock)?);
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert_close(report.values.illuminance, 512.0, 0.5);
        Ok(())
    }

    #[test]
    fn a_temperature_tag_at_0x49_next_to_a_humidity_tag() -> Result<(), BusError> {
        let tag: SharedEnv = Arc::new(Mutex::new(Values {
            temperature: Some(5.5),
            humidity: Some(80.0),
            ..Values::default()
        }));
        let mut bus = MockBus::new();
        bus.attach(0, 0x49, Box::new(SimTmp112::new(tag.clone())));
        bus.attach(0, 0x40, Box::new(SimSht20::new(tag)));
        let clock = MockClock::new();
        let mut sensors = Sensors::new(ChannelMode::Channel0);
        sensors.detect(&mut bus, &clock)?;
        // Not a Climate Module R1: its TMP112 sits at 0x48.
        assert_eq!(
            modules(&sensors),
            vec![
                (0, ModuleKind::HumidityTag),
                (0, ModuleKind::TemperatureTag)
            ]
        );
        let t = sensors
            .sources()
            .into_iter()
            .find(|s| s.metric == Metric::Temperature);
        assert_eq!(
            t.map(|s| (s.module, s.sensor, s.address)),
            Some((ModuleKind::TemperatureTag, "TMP112".to_string(), 0x49))
        );
        let report = single(sensors.measure(&mut bus, &clock)?);
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert_close(report.values.temperature, 5.5, 0.07);
        assert_close(report.values.humidity, 80.0, 0.1);
        Ok(())
    }

    #[test]
    fn a_lux_meter_tag_at_0x45_next_to_a_climate_module_r1() -> Result<(), BusError> {
        let mut bus = MockBus::new();
        bus.attach(0, 0x48, Box::new(SimTmp112::new(env())));
        bus.attach(0, 0x40, Box::new(SimSht20::new(env())));
        bus.attach(0, 0x45, Box::new(SimOpt3001::new(env())));
        let clock = MockClock::new();
        let mut sensors = Sensors::new(ChannelMode::Channel0);
        sensors.detect(&mut bus, &clock)?;
        assert_eq!(
            modules(&sensors),
            vec![
                (0, ModuleKind::ClimateModuleR1),
                (0, ModuleKind::LuxMeterTag)
            ]
        );
        Ok(())
    }

    #[test]
    fn the_default_address_wins_when_both_answer() -> Result<(), BusError> {
        let mut bus = MockBus::with_climate(ClimateRevision::R1, 0, env());
        bus.attach(0, 0x49, Box::new(SimTmp112::new(env())));
        let clock = MockClock::new();
        let mut sensors = Sensors::new(ChannelMode::Channel0);
        sensors.detect(&mut bus, &clock)?;
        let t = sensors
            .sources()
            .into_iter()
            .find(|s| s.metric == Metric::Temperature);
        assert_eq!(
            t.map(|s| (s.module, s.address)),
            Some((ModuleKind::ClimateModuleR1, 0x48))
        );
        Ok(())
    }

    #[test]
    fn channel_mode_limits_the_search() -> Result<(), BusError> {
        let mut bus = MockBus::with_climate(ClimateRevision::R2, 0, env());
        attach_tags(&mut bus, 1, env());
        let clock = MockClock::new();
        let mut sensors = Sensors::new(ChannelMode::Channel1);
        sensors.detect(&mut bus, &clock)?;
        assert!(sensors.sources().iter().all(|s| s.channel == 1));
        Ok(())
    }

    #[test]
    fn sources_name_the_sensor_and_address_per_value() -> Result<(), BusError> {
        let clock = MockClock::new();
        let mut r1 = Sensors::new(ChannelMode::Auto);
        r1.detect(
            &mut MockBus::with_climate(ClimateRevision::R1, 0, env()),
            &clock,
        )?;
        let t = r1
            .sources()
            .into_iter()
            .find(|s| s.metric == Metric::Temperature);
        assert_eq!(
            t.map(|s| (s.sensor, s.address)),
            Some(("TMP112".to_string(), 0x48))
        );
        let mut r2 = Sensors::new(ChannelMode::Auto);
        r2.detect(
            &mut MockBus::with_climate(ClimateRevision::R2, 0, env()),
            &clock,
        )?;
        let sources = r2.sources();
        assert_eq!(sources.iter().filter(|s| s.sensor == "SHT30").count(), 2);
        assert_eq!(sources.len(), 4);
        Ok(())
    }

    #[test]
    fn no_module() -> Result<(), BusError> {
        let mut bus = MockBus::new();
        let clock = MockClock::new();
        let mut sensors = Sensors::new(ChannelMode::Auto);
        assert!(!sensors.detect(&mut bus, &clock)?);
        assert!(sensors.is_empty());
        Ok(())
    }

    #[test]
    fn steady_state_cycle_is_about_one_second() -> Result<(), BusError> {
        let mut bus = MockBus::with_climate(ClimateRevision::R2, 0, env());
        let clock = MockClock::new();
        let mut sensors = Sensors::new(ChannelMode::Channel0);
        sensors.detect(&mut bus, &clock)?;
        sensors.measure(&mut bus, &clock)?; // first cycle includes the barometer reset
        let report = single(sensors.measure(&mut bus, &clock)?);
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
        let mut sensors = Sensors::new(ChannelMode::Channel0);
        sensors.detect(&mut bus, &clock)?;
        sensors.measure(&mut bus, &clock)?;
        bus.log.clear();
        sensors.measure(&mut bus, &clock)?;
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
        let mut sensors = Sensors::new(ChannelMode::Channel0);
        sensors.detect(&mut bus, &clock)?;
        sensors.measure(&mut bus, &clock)?;
        // Fail the SHT30 trigger (second transaction after the switch write).
        bus.inject = vec![];
        bus.log.clear();
        let mut faulty = FailNth {
            inner: &mut bus,
            n: 1,
        };
        let report = single(sensors.measure(&mut faulty, &clock)?);
        assert_eq!(report.values.temperature, None);
        assert_eq!(report.errors.len(), 1);
        assert!(report.values.pressure.is_some());
        bus.log.clear();
        let report = single(sensors.measure(&mut bus, &clock)?);
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
        let mut sensors = Sensors::new(ChannelMode::Channel0);
        sensors.detect(&mut bus, &clock)?;
        bus.inject = vec![BusError::Hid("gone".into())];
        assert!(matches!(
            sensors.measure(&mut bus, &clock),
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
