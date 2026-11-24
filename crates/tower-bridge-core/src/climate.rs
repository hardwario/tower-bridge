//! Sensors behind the Bridge Module's I2C switch: the TOWER Climate Module
//! and the TOWER sensor tags, on either switch channel.
//!
//! Parts and addresses (see twr-sdk):
//! - SHT30 @0x45: temperature and humidity (Climate Module R2; a Humidity
//!   Tag R4 at 0x44, or 0x45)
//! - SHT20 @0x40: humidity, and temperature on its own (Climate Module R1,
//!   Humidity Tag R3)
//! - TMP112 @0x48: temperature (Climate Module R1, Temperature Tag; the tag
//!   can also sit at 0x49)
//! - OPT3001 @0x44: illuminance (Climate Module, Lux Meter Tag; the tag can
//!   also sit at 0x45)
//! - MPL3115A2 @0x60: pressure (Climate Module, Barometer Tag)
//! - LP8 behind an SC16IS740 @0x4D: CO2 (CO2 Module, see [`crate::co2`])
//! - SGPC3 @0x58: TVOC (VOC-LP Tag)
//!
//! Every channel is probed part by part, at every address a part can have,
//! and every part found is measured. The parts are then put together into
//! modules ([`ChannelSensors::modules`]): a Climate Module is recognised by
//! its parts at their default addresses, any other part is named after the
//! tag that carries it. Within a module each metric has one part (a Climate
//! Module R1 measures temperature with its TMP112, not its SHT20).
//!
//! The first module of a channel that measures a metric is the channel's
//! primary source of it, whose values go into the channel's reading; a
//! Climate Module comes first, then the tags in the order SHT30, TMP112,
//! SHT20. Every other module measuring the same metric on the channel (a
//! Humidity Tag next to a Climate Module) reports in a reading of its own,
//! named by its part and address (see [`crate::model::sensor_id`]).
//!
//! A measurement cycle triggers every conversion on a channel first and then
//! collects the results as they become ready, so one channel takes about one
//! second (the OPT3001 800 ms conversion dominates). Channels are measured
//! one after the other, because the same addresses are used on both.
//!
//! CO2 and TVOC keep their own pace: the LP8 is measured at the end of its
//! channel's cycle once 16 s have passed since the last time, and the SGPC3
//! every 30 s between the cycles ([`Sensors::service`]); a cycle reports
//! their latest values.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::bus::{BusError, Clock, I2cBus};
use crate::co2::{self, Co2Module};
use crate::model::{
    BusWarning, BusWarningKind, Metric, ModuleKind, SensorSource, Values, sensor_id,
};
use crate::sensors::sgpc3::{self, Sgpc3};
use crate::sensors::{SensorError, mpl3115a2, opt3001, sht20, sht30, tmp112};

/// A CO2 value is reported for this long after its measurement (the LP8
/// measures at most every 16 s).
const CO2_MAX_AGE: Duration = Duration::from_secs(60);
/// The SGPC3 measures every 30 s.
const VOC_MAX_AGE: Duration = Duration::from_secs(90);

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

/// How the parts of a Climate Module R1 are named on a channel. The R1 has
/// no part of its own to recognise it by: a Humidity, Temperature, Lux Meter
/// and Barometer Tag at their default addresses look exactly the same.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClimateR1 {
    /// A Climate Module R1 when all four of its parts answer (SHT20 0x40,
    /// TMP112 0x48, OPT3001 0x44, MPL3115A2 0x60), else tags. Once
    /// recognised, it stays a module while its SHT20 and TMP112 answer.
    #[default]
    Auto,
    /// A Climate Module R1 whenever its SHT20 and TMP112 answer.
    Module,
    /// Always tags.
    Tags,
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

    /// Every address the part answers at on a TOWER module or tag, the
    /// Climate Module's first.
    pub fn addresses(self) -> &'static [u8] {
        match self {
            SensorKind::Sht30 => &[sht30::ADDR, sht30::TAG_ADDR],
            SensorKind::Sht20 => &[sht20::ADDR],
            SensorKind::Tmp112 => &[tmp112::ADDR, tmp112::ALT_ADDR],
            SensorKind::Opt3001 => &[opt3001::ADDR, opt3001::ALT_ADDR],
            SensorKind::Mpl3115a2 => &[mpl3115a2::ADDR],
        }
    }

    /// The tag that carries the part on its own.
    fn tag(self) -> ModuleKind {
        match self {
            SensorKind::Sht30 | SensorKind::Sht20 => ModuleKind::HumidityTag,
            SensorKind::Tmp112 => ModuleKind::TemperatureTag,
            SensorKind::Opt3001 => ModuleKind::LuxMeterTag,
            SensorKind::Mpl3115a2 => ModuleKind::BarometerTag,
        }
    }

    /// What the part measures on its own (on a tag).
    fn metrics(self) -> &'static [Metric] {
        match self {
            SensorKind::Sht30 | SensorKind::Sht20 => &[Metric::Temperature, Metric::Humidity],
            SensorKind::Tmp112 => &[Metric::Temperature],
            SensorKind::Opt3001 => &[Metric::Illuminance],
            SensorKind::Mpl3115a2 => &[Metric::Pressure],
        }
    }
}

/// The order in which tags become a channel's primary source: the SHT30
/// before the TMP112 before the SHT20 (as the Climate Modules measure
/// temperature), then the light sensor and the barometer.
const TAG_ORDER: [SensorKind; 5] = [
    SensorKind::Sht30,
    SensorKind::Tmp112,
    SensorKind::Sht20,
    SensorKind::Opt3001,
    SensorKind::Mpl3115a2,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slot {
    Absent,
    Uninit,
    Ready,
}

/// One sensor part on a channel.
#[derive(Clone, Copy, Debug)]
struct Part {
    kind: SensorKind,
    slot: Slot,
    /// Failed measurements in a row.
    fails: u8,
    /// 7-bit I2C address it answered at.
    addr: u8,
}

impl Part {
    fn present(self) -> bool {
        self.slot != Slot::Absent
    }

    fn ready(self) -> bool {
        self.slot == Slot::Ready
    }

    fn found(kind: SensorKind, slot: Slot, addr: u8) -> Part {
        Part {
            kind,
            slot,
            fails: 0,
            addr,
        }
    }

    fn id(self) -> String {
        sensor_id(self.kind.name(), self.addr)
    }
}

/// The parts of one module or tag on a channel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Module {
    pub kind: ModuleKind,
    /// The part measuring each metric, as an index into the channel's parts.
    parts: Vec<(Metric, usize)>,
}

/// A part that fails this many measurements in a row is taken as unplugged;
/// the periodic probe finds it again when it comes back.
const PART_GIVE_UP: u8 = 3;

/// Suspicion of an address collision from what a cycle shows (a checksum
/// error, a Climate Module's barometer far off its thermometer): a suspicious
/// cycle adds [`SUSPECT_STEP`], a clean one takes one away. At
/// [`SUSPECT_AT`] (two suspicious cycles close together) the part is
/// reported, and it stays reported until the score is back at zero, so a
/// warning does not come and go around one level.
const SUSPECT_STEP: u8 = 3;
const SUSPECT_AT: u8 = 6;
const SUSPECT_MAX: u8 = 12;
/// What is known about a part that stopped answering is kept this long:
/// two devices at one address garble each other's checksums until the part
/// is given up, and the probe finds it again within a minute. Counting
/// cycles instead let the warning go between two probes at a short interval.
const SUSPECT_HOLD: Duration = Duration::from_secs(300);

/// How many changed readings [`LowBits`] remembers, to see whether a part's
/// readings vary.
const LOW_BITS_RECENT: usize = 32;
/// A reading can only add to the score while this many different values are
/// among the remembered ones: a lone sensor whose reading flips between a few
/// values, one of them a multiple of eight, would otherwise count that one
/// value over and over. With twelve codes in play at most three in seventeen
/// are multiples of eight (18 %).
const LOW_BITS_VARIETY: usize = 12;
/// A changed reading with its three lowest bits all zero adds this to the
/// score, any other takes one away: the score falls below one zero in four.
/// A lone sensor has its lowest bits at random, all three zero in one reading
/// of eight, and loses half a point a reading. Two sensors at one address
/// are read ANDed bit by bit: a bit is zero three times in four, all three
/// in (3/4)³ = 42 % of the readings, 0.7 points up a reading.
///
/// Measured on 2026-11-23 at a 2 s interval (Climate Module R2.1, tags):
/// 12 to 14.5 % alone (two barometers, two light sensors, a TMP112, 71
/// minutes each), 40 to 42 % together (a barometer pair, a light sensor
/// pair). Alone the score stayed below 24 there, and below 51 in 1200
/// simulated days of the worst case (a sensor spread evenly over 17 codes
/// that start at a multiple of eight).
const LOW_BITS_STEP: u8 = 3;
/// The score from which a part is suspect; a pair gets there in about 150
/// changed readings.
const LOW_BITS_SUSPECT: u8 = 72;
/// The score down to which a suspect part stays one: a pair does not drop
/// out on a run of other readings, and a part left alone is cleared about
/// 65 changed readings later.
const LOW_BITS_CLEAR: u8 = 48;
const LOW_BITS_MAX: u8 = 80;

/// Watches the lowest bits of a part's raw readings for a second device at
/// its address (see [`LOW_BITS_STEP`]). Only changed readings count.
///
/// An earlier version counted the zeros among the last 90 changed readings
/// (22 or more made the part suspect). A lone sensor gets there by chance in
/// one window of 700, and a window was judged at every reading: the clean
/// set-up of 2026-11-23 warned five times in 70 minutes.
#[derive(Clone, Debug, Default)]
struct LowBits {
    /// The last changed readings, newest last.
    recent: VecDeque<u32>,
    score: u8,
    suspect: bool,
}

impl LowBits {
    /// Note a raw reading; whether the readings so far suggest a second
    /// device.
    fn push(&mut self, raw: u32) -> bool {
        if self.recent.back() == Some(&raw) {
            return self.suspect;
        }
        self.recent.push_back(raw);
        if self.recent.len() > LOW_BITS_RECENT {
            self.recent.pop_front();
        }
        let varied = self.recent.len() == LOW_BITS_RECENT && {
            let mut distinct: Vec<u32> = self.recent.iter().copied().collect();
            distinct.sort_unstable();
            distinct.dedup();
            distinct.len() >= LOW_BITS_VARIETY
        };
        // Without variety nothing can be told; what was gathered fades, so
        // a warning ends once its cause is gone and the reading steady.
        self.score = if varied && raw & 0b111 == 0 {
            self.score.saturating_add(LOW_BITS_STEP).min(LOW_BITS_MAX)
        } else {
            self.score.saturating_sub(1)
        };
        if self.score >= LOW_BITS_SUSPECT {
            self.suspect = true;
        } else if self.score <= LOW_BITS_CLEAR {
            self.suspect = false;
        }
        self.suspect
    }
}

/// What the readings of one part tell about a second device at its address.
#[derive(Clone, Debug)]
struct Watch {
    kind: SensorKind,
    addr: u8,
    /// See [`SUSPECT_STEP`].
    score: u8,
    /// The score reached [`SUSPECT_AT`] and is not back at zero yet.
    reported: bool,
    /// The lowest bits of the raw readings, for the parts without a
    /// checksum.
    low_bits: LowBits,
    /// Since when the part does not answer, while it does not.
    away_since: Option<Duration>,
}

impl Watch {
    fn new(kind: SensorKind, addr: u8) -> Self {
        Self {
            kind,
            addr,
            score: 0,
            reported: false,
            low_bits: LowBits::default(),
            away_since: None,
        }
    }

    /// Nothing gathered: not worth keeping.
    fn idle(&self) -> bool {
        self.score == 0 && self.low_bits.recent.is_empty()
    }
}

/// A Climate Module's barometer differs this much from its thermometer only
/// when a second barometer answers at its address (its die runs a degree or
/// two warm).
const BAROMETER_MISMATCH_C: f64 = 6.0;

/// Result of measuring one channel.
#[derive(Clone, Debug, Default)]
pub struct CycleReport {
    /// The channel's values, from the primary source of each metric.
    pub values: Values,
    /// The values of the channel's further sensors, by sensor id (see
    /// [`crate::model::sensor_id`]); only sensors with a value.
    pub extra: Vec<(String, Values)>,
    pub errors: Vec<(SensorKind, SensorError)>,
    pub duration: Duration,
    /// The switch did not select the channel, or lost it during the cycle:
    /// nothing could be measured. A failed cycle, unlike one that simply
    /// has no value yet (a CO2 Module charging).
    pub unreachable: bool,
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
    /// Every part found, in the order found; a part given up is marked
    /// absent during a cycle and removed at its end.
    parts: Vec<Part>,
    co2: Option<Co2Module>,
    voc: Option<Sgpc3>,
    /// How the parts at the Climate Module R1 addresses are named.
    r1: ClimateR1,
    /// All four parts of a Climate Module R1 were seen together (see
    /// [`ClimateR1::Auto`]).
    r1_seen: bool,
    /// What suggests a second device at a part's address, by part and
    /// address; kept for [`SUSPECT_HOLD`] while the part is away, so a part
    /// dropped for its garbled data stays reported.
    watched: Vec<Watch>,
    /// Modules kept from before a re-detection (Rescan, a channel change):
    /// taken over if found again, instead of starting them anew (a minute
    /// of charging for the LP8, a new preheat for the SGPC3 that loses what
    /// it learnt since its baseline was stored).
    spare_co2: Option<Co2Module>,
    spare_voc: Option<Sgpc3>,
    /// Where the LP8 and SGPC3 keep their state between runs.
    state_dir: Option<PathBuf>,
}

impl ChannelSensors {
    fn new(channel: u8, state_dir: Option<PathBuf>, r1: ClimateR1) -> Self {
        Self {
            channel,
            parts: Vec::new(),
            co2: None,
            voc: None,
            r1,
            r1_seen: false,
            watched: Vec::new(),
            spare_co2: None,
            spare_voc: None,
            state_dir,
        }
    }

    fn state_file(&self, name: &str) -> Option<PathBuf> {
        self.state_dir
            .as_ref()
            .map(|d| d.join(format!("{name}-channel{}.txt", self.channel)))
    }

    pub fn channel(&self) -> u8 {
        self.channel
    }

    pub fn is_empty(&self) -> bool {
        !(self.parts.iter().any(|p| p.present()) || self.co2.is_some() || self.voc.is_some())
    }

    /// The index of `kind` at `addr`, when present.
    fn find(&self, kind: SensorKind, addr: u8) -> Option<usize> {
        self.parts
            .iter()
            .position(|p| p.present() && p.kind == kind && p.addr == addr)
    }

    /// Whether a part was found at `addr`.
    fn holds(&self, addr: u8) -> bool {
        self.parts.iter().any(|p| p.present() && p.addr == addr)
    }

    fn add(&mut self, kind: SensorKind, slot: Slot, addr: u8) {
        self.parts.push(Part::found(kind, slot, addr));
    }

    /// Look for the parts not present yet (the channel must be selected).
    /// Returns whether one was found.
    fn probe(&mut self, bus: &mut dyn I2cBus, clock: &dyn Clock) -> Result<bool, BusError> {
        let mut found = false;
        // 0x44 and 0x45 hold an OPT3001 or an SHT30. The light sensor first,
        // by its ID: an OPT3001 would also acknowledge the SHT30 soft reset.
        // SHT humidity sensors only acknowledge commands, so probe them with
        // a soft reset (an idle SHT3x does not even acknowledge a read).
        for addr in [opt3001::ADDR, opt3001::ALT_ADDR] {
            if self.holds(addr) {
                continue;
            }
            if answered(opt3001::device_id(bus, addr))? == Some(opt3001::DEVICE_ID) {
                self.add(SensorKind::Opt3001, Slot::Uninit, addr);
                found = true;
            } else if answered(sht30::soft_reset(bus, addr))?.is_some() {
                clock.sleep(sht30::RESET_TIME);
                self.add(SensorKind::Sht30, Slot::Ready, addr);
                found = true;
            }
        }
        // An HDC2080 at the same address would take the SHT20's commands as
        // register pointers and read back nonsense (-46.85 °C from zeros).
        if !self.holds(sht20::ADDR)
            && !crate::sensors::identify::is_hdc2080(bus, sht20::ADDR)?
            && answered(sht20::soft_reset(bus))?.is_some()
        {
            clock.sleep(sht20::RESET_TIME);
            self.add(SensorKind::Sht20, Slot::Ready, sht20::ADDR);
            found = true;
        }
        for addr in [tmp112::ADDR, tmp112::ALT_ADDR] {
            if !self.holds(addr) && answered(tmp112::probe(bus, addr))?.is_some() {
                self.add(SensorKind::Tmp112, Slot::Ready, addr);
                found = true;
            }
        }
        if !self.holds(mpl3115a2::ADDR)
            && answered(mpl3115a2::who_am_i(bus))? == Some(mpl3115a2::WHO_AM_I)
        {
            self.add(SensorKind::Mpl3115a2, Slot::Uninit, mpl3115a2::ADDR);
            found = true;
        }
        if self.voc.is_none() && sgpc3::probe(bus)?.is_some() {
            self.voc = match self.spare_voc.take() {
                Some(voc) => Some(voc),
                None => {
                    let store = self.state_file("sgpc3");
                    answered(Sgpc3::start(bus, clock, self.channel, store))?
                }
            };
            found |= self.voc.is_some();
        }
        if self.co2.is_none() && Co2Module::probe(bus)? {
            self.co2 = match self.spare_co2.take() {
                Some(co2) => Some(co2),
                None => {
                    let store = self.state_file("lp8");
                    answered(Co2Module::start(bus, clock, self.channel, store))?
                }
            };
            found |= self.co2.is_some();
        }
        // Measured in this order, as the drivers expect (see the cycle).
        self.parts.sort_by_key(|p| {
            let kind = TAG_ORDER.iter().position(|k| *k == p.kind);
            let addr = p.kind.addresses().iter().position(|a| *a == p.addr);
            (kind, addr)
        });
        self.note_r1();
        Ok(found)
    }

    /// Remember a complete Climate Module R1, and forget it once its SHT20
    /// or TMP112 is gone (see [`ClimateR1::Auto`]).
    fn note_r1(&mut self) {
        let thermometers = self.find(SensorKind::Sht20, sht20::ADDR).is_some()
            && self.find(SensorKind::Tmp112, tmp112::ADDR).is_some();
        let complete = thermometers
            && self.find(SensorKind::Opt3001, opt3001::ADDR).is_some()
            && self.find(SensorKind::Mpl3115a2, mpl3115a2::ADDR).is_some();
        self.r1_seen = thermometers && (self.r1_seen || complete);
    }

    /// The modules and tags on this channel, Climate Module first, then the
    /// tags in [`TAG_ORDER`] and by address: the first that measures a
    /// metric is the channel's primary source of it.
    pub fn modules(&self) -> Vec<Module> {
        let at = |kind: SensorKind, addr: u8| self.find(kind, addr);
        let mut modules = Vec::new();
        let lux = at(SensorKind::Opt3001, opt3001::ADDR);
        let baro = at(SensorKind::Mpl3115a2, mpl3115a2::ADDR);
        let climate_extras = || {
            lux.map(|i| (Metric::Illuminance, i))
                .into_iter()
                .chain(baro.map(|i| (Metric::Pressure, i)))
        };
        let sht30 = at(SensorKind::Sht30, sht30::ADDR);
        let sht20 = at(SensorKind::Sht20, sht20::ADDR);
        let tmp112 = at(SensorKind::Tmp112, tmp112::ADDR);
        if let Some(s) = sht30
            && (lux.is_some() || baro.is_some())
        {
            let mut parts = vec![(Metric::Temperature, s), (Metric::Humidity, s)];
            parts.extend(climate_extras());
            modules.push(Module {
                kind: ModuleKind::ClimateModuleR2,
                parts,
            });
        } else if let (Some(h), Some(t)) = (sht20, tmp112) {
            let r1 = match self.r1 {
                ClimateR1::Module => true,
                ClimateR1::Tags => false,
                ClimateR1::Auto => self.r1_seen || (lux.is_some() && baro.is_some()),
            };
            if r1 {
                let mut parts = vec![(Metric::Temperature, t), (Metric::Humidity, h)];
                parts.extend(climate_extras());
                modules.push(Module {
                    kind: ModuleKind::ClimateModuleR1,
                    parts,
                });
            }
        }
        let used: Vec<usize> = modules
            .iter()
            .flat_map(|m| m.parts.iter().map(|(_, i)| *i))
            .collect();
        for kind in TAG_ORDER {
            for &addr in kind.addresses() {
                if let Some(i) = at(kind, addr)
                    && !used.contains(&i)
                {
                    modules.push(Module {
                        kind: kind.tag(),
                        parts: kind.metrics().iter().map(|m| (*m, i)).collect(),
                    });
                }
            }
        }
        modules
    }

    pub fn sources(&self) -> Vec<SensorSource> {
        self.sources_of(&self.modules())
    }

    /// The source of each metric of each module, by metric; the first of a
    /// metric is the primary one.
    fn sources_of(&self, modules: &[Module]) -> Vec<SensorSource> {
        let mut out = Vec::new();
        for metric in [
            Metric::Temperature,
            Metric::Humidity,
            Metric::Illuminance,
            Metric::Pressure,
        ] {
            let mut primary = true;
            for module in modules {
                for &(m, i) in &module.parts {
                    if m != metric {
                        continue;
                    }
                    let part = self.parts[i];
                    out.push(SensorSource {
                        metric,
                        channel: self.channel,
                        module: module.kind,
                        sensor: part.kind.name().into(),
                        address: part.addr,
                        id: part.id(),
                        primary,
                    });
                    primary = false;
                }
            }
        }
        if self.co2.is_some() {
            out.push(SensorSource {
                metric: Metric::Co2,
                channel: self.channel,
                module: ModuleKind::Co2Module,
                sensor: "LP8".into(),
                address: co2::UART,
                id: sensor_id("LP8", co2::UART),
                primary: true,
            });
        }
        if self.voc.is_some() {
            out.push(SensorSource {
                metric: Metric::Voc,
                channel: self.channel,
                module: ModuleKind::VocLpTag,
                sensor: "SGPC3".into(),
                address: sgpc3::ADDR,
                id: sensor_id("SGPC3", sgpc3::ADDR),
                primary: true,
            });
        }
        out
    }

    /// Measure TVOC when due (the channel must be selected); `climate` is
    /// this channel's temperature and humidity, for compensation.
    fn service(
        &mut self,
        bus: &mut dyn I2cBus,
        clock: &dyn Clock,
        climate: Option<(f64, f64)>,
    ) -> Result<(), BusError> {
        if let Some(voc) = self.voc.as_mut() {
            voc.service(bus, clock, climate)?;
            if voc.gone() {
                info!("VOC-LP Tag on channel {} stopped answering", self.channel);
                self.voc = None;
            }
        }
        Ok(())
    }

    /// The TVOC slot could not be served.
    fn missed(&mut self, now: Duration) {
        if let Some(voc) = self.voc.as_mut() {
            voc.missed(now);
            if voc.gone() {
                info!("VOC-LP Tag on channel {} stopped answering", self.channel);
                self.voc = None;
            }
        }
    }

    /// A measurement of part `i` succeeded: its failures in a row start over.
    fn measured(&mut self, i: usize) {
        self.parts[i].fails = 0;
        let part = self.parts[i];
        self.suspect(part.kind, part.addr, false);
    }

    fn watch_index(&self, kind: SensorKind, addr: u8) -> Option<usize> {
        self.watched
            .iter()
            .position(|w| w.kind == kind && w.addr == addr)
    }

    /// The watch of `kind` at `addr`, started if there is none.
    fn watch(&mut self, kind: SensorKind, addr: u8) -> &mut Watch {
        let i = match self.watch_index(kind, addr) {
            Some(i) => i,
            None => {
                self.watched.push(Watch::new(kind, addr));
                self.watched.len() - 1
            }
        };
        &mut self.watched[i]
    }

    /// Note a cycle of `kind` at `addr` that suggests a second device at its
    /// address (`true`), or one that does not.
    fn suspect(&mut self, kind: SensorKind, addr: u8, suspicious: bool) {
        if suspicious {
            let w = self.watch(kind, addr);
            w.score = w.score.saturating_add(SUSPECT_STEP).min(SUSPECT_MAX);
            w.reported |= w.score >= SUSPECT_AT;
        } else if let Some(i) = self.watch_index(kind, addr) {
            let w = &mut self.watched[i];
            w.score = w.score.saturating_sub(1);
            w.reported &= w.score > 0;
        }
    }

    /// Note a raw reading of part `i` (without a checksum), for a second
    /// device that shows in its lowest bits (see [`LowBits`]).
    fn watch_low_bits(&mut self, i: usize, raw: u32) {
        let Part { kind, addr, .. } = self.parts[i];
        self.watch(kind, addr).low_bits.push(raw);
    }

    /// At the end of a cycle: forget what is known about parts away for
    /// longer than [`SUSPECT_HOLD`], and watches with nothing in them.
    fn tidy_watched(&mut self, now: Duration) {
        let present: Vec<bool> = self
            .watched
            .iter()
            .map(|w| self.find(w.kind, w.addr).is_some())
            .collect();
        for (w, present) in self.watched.iter_mut().zip(present) {
            w.away_since = match (present, w.away_since) {
                (true, _) => None,
                (false, None) => Some(now),
                (false, since) => since,
            };
        }
        self.watched.retain(|w| {
            let held = w
                .away_since
                .is_none_or(|since| now.saturating_sub(since) < SUSPECT_HOLD);
            held && !w.idle()
        });
    }

    /// The parts whose data suggest a second device at their address.
    pub fn warnings(&self) -> Vec<BusWarning> {
        let modules = self.modules();
        let mut out: Vec<BusWarning> = self
            .watched
            .iter()
            // What the lowest bits told counts while the part answers:
            // unlike garbled checksums, it does not make a part go away.
            .filter(|w| w.reported || (w.low_bits.suspect && w.away_since.is_none()))
            .map(|w| {
                let (kind, addr) = (w.kind, w.addr);
                let module = self
                    .find(kind, addr)
                    .and_then(|i| {
                        modules
                            .iter()
                            .find(|m| m.parts.iter().any(|(_, p)| *p == i))
                    })
                    .map_or(kind.tag(), |m| m.kind);
                BusWarning {
                    channel: self.channel,
                    address: addr,
                    sensor: kind.name().into(),
                    module,
                    kind: BusWarningKind::AddressCollision,
                }
            })
            .collect();
        out.sort_by_key(|w| w.address);
        out
    }

    /// Sort out part `i`'s result: failures are recorded and the part is set
    /// up again next time, or given up after [`PART_GIVE_UP`] failed
    /// measurements in a row (a set-up that works does not reset the count;
    /// see [`measured`](Self::measured)). `Err` only for transport failures.
    fn check<T>(
        &mut self,
        errors: &mut Vec<(SensorKind, SensorError)>,
        i: usize,
        r: Result<T, SensorError>,
    ) -> Result<Option<T>, BusError> {
        let channel = self.channel;
        let part = &mut self.parts[i];
        match r {
            Ok(v) => Ok(Some(v)),
            Err(SensorError::Bus(e)) if e.is_fatal() => Err(e),
            Err(e) => {
                let name = part.kind.name();
                let addr = part.addr;
                // Two devices answering one read garble its checksum; a
                // lone sensor almost never does.
                let garbled = matches!(e, SensorError::Crc);
                debug!("{name} at 0x{addr:02X} on channel {channel} failed: {e}");
                part.fails = part.fails.saturating_add(1);
                if part.fails >= PART_GIVE_UP {
                    info!("{name} at 0x{addr:02X} on channel {channel} stopped answering");
                    part.slot = Slot::Absent;
                } else {
                    part.slot = Slot::Uninit;
                }
                let kind = part.kind;
                errors.push((kind, e));
                if garbled {
                    self.suspect(kind, addr, true);
                }
                Ok(None)
            }
        }
    }

    /// The indices of the ready parts of `kind`.
    fn ready(&self, kind: SensorKind) -> Vec<usize> {
        (0..self.parts.len())
            .filter(|&i| self.parts[i].kind == kind && self.parts[i].ready())
            .collect()
    }

    /// Measure this channel; see [`Sensors::measure_with`]. `pressure_hint`
    /// is the ambient pressure measured elsewhere, for the CO2 compensation.
    fn measure_with(
        &mut self,
        bus: &mut dyn I2cBus,
        clock: &dyn Clock,
        idle: &mut dyn FnMut(&mut dyn I2cBus, Duration) -> bool,
        pressure_hint: Option<f64>,
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
                report.unreachable = true;
                return Ok(report);
            }
        }
        // Who measures what is decided at the start of the cycle; a part
        // given up during it is left out from the next cycle on.
        let modules = self.modules();
        let sources = self.sources_of(&modules);
        // An SHT20 measures temperature when a tag carries it, not on a
        // Climate Module R1 (whose TMP112 does).
        let sht20_temperature = modules.iter().any(|m| {
            m.parts.iter().any(|&(metric, i)| {
                metric == Metric::Temperature && self.parts[i].kind == SensorKind::Sht20
            })
        });
        let errors = &mut report.errors;
        let mut results = vec![Values::default(); self.parts.len()];

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
        for i in 0..self.parts.len() {
            let part = self.parts[i];
            if part.slot != Slot::Uninit {
                continue;
            }
            let r = match part.kind {
                SensorKind::Sht30 => sht30::soft_reset(bus, part.addr).map(|()| sht30::RESET_TIME),
                SensorKind::Sht20 => sht20::soft_reset(bus).map(|()| sht20::RESET_TIME),
                SensorKind::Tmp112 => Ok(Duration::ZERO),
                SensorKind::Opt3001 => opt3001::init(bus, part.addr).map(|()| Duration::ZERO),
                // Below: its reset takes a second.
                SensorKind::Mpl3115a2 => continue,
            };
            if let Some(wait) = self.check(errors, i, r.map_err(SensorError::from))? {
                clock.sleep(wait);
                self.parts[i].slot = Slot::Ready;
            }
        }
        if let Some(i) = (0..self.parts.len()).find(|&i| {
            self.parts[i].kind == SensorKind::Mpl3115a2 && self.parts[i].slot == Slot::Uninit
        }) {
            mpl3115a2::reset(bus);
            // A second of reset: keep the relay responsive meanwhile.
            on_channel = idle_until(bus, clock.now() + mpl3115a2::RESET_TIME)?;
            if on_channel {
                let r = mpl3115a2::configure(bus).map_err(SensorError::from);
                if self.check(errors, i, r)?.is_some() {
                    self.parts[i].slot = Slot::Ready;
                }
            }
        }
        if !on_channel {
            report.duration = clock.now() - started;
            report.unreachable = true;
            return Ok(report);
        }

        // Trigger all conversions.
        let t0 = clock.now();
        for i in 0..self.parts.len() {
            let part = self.parts[i];
            if !part.ready() {
                continue;
            }
            let r = match part.kind {
                SensorKind::Sht30 => sht30::trigger(bus, part.addr),
                SensorKind::Tmp112 => tmp112::trigger(bus, part.addr),
                // The SHT20 converts one quantity at a time: temperature
                // first when it measures it, humidity right after.
                SensorKind::Sht20 if sht20_temperature => sht20::trigger_temperature(bus),
                SensorKind::Sht20 => sht20::trigger_humidity(bus),
                SensorKind::Opt3001 => opt3001::trigger(bus, part.addr),
                SensorKind::Mpl3115a2 => mpl3115a2::trigger(bus),
            };
            self.check(errors, i, r.map_err(SensorError::from))?;
        }

        // Fast sensors.
        clock.sleep_until(t0 + FAST_FETCH_AT);
        for i in self.ready(SensorKind::Sht30) {
            let r = sht30::fetch(bus, self.parts[i].addr);
            if let Some((t, rh)) = self.check(errors, i, r)? {
                self.measured(i);
                results[i].temperature = Some(t);
                results[i].humidity = Some(rh.clamp(0.0, 100.0));
            }
        }
        for i in self.ready(SensorKind::Tmp112) {
            let addr = self.parts[i].addr;
            let r = poll_ready(clock, || tmp112::is_ready(bus, addr), 3, SHT20_RETRY)
                .and_then(|()| tmp112::fetch(bus, addr));
            if let Some(t) = self.check(errors, i, r)? {
                self.measured(i);
                results[i].temperature = Some(t);
                // The 12-bit register: steps of 0.0625 °C.
                self.watch_low_bits(i, ((t / 0.0625).round() as i32 & 0x0FFF) as u32);
            }
        }
        for i in self.ready(SensorKind::Sht20) {
            if sht20_temperature {
                let r = sht20_fetch(clock, || sht20::fetch_temperature(bus));
                if let Some(t) = self.check(errors, i, r)? {
                    self.measured(i);
                    results[i].temperature = Some(t);
                }
                if self.parts[i].ready() {
                    let r = sht20::trigger_humidity(bus).map_err(SensorError::from);
                    if self.check(errors, i, r)?.is_some() {
                        clock.sleep(sht20::HUMIDITY_TIME);
                    }
                }
            }
            if self.parts[i].ready() {
                let r = sht20_fetch(clock, || sht20::fetch_humidity(bus));
                if let Some(rh) = self.check(errors, i, r)? {
                    self.measured(i);
                    results[i].humidity = Some(rh.clamp(0.0, 100.0));
                }
            }
        }

        // Barometer (waiting for the slow sensors goes to the idle hook).
        let barometers = self.ready(SensorKind::Mpl3115a2);
        if !barometers.is_empty() {
            on_channel = idle_until(bus, t0 + mpl3115a2::CONVERSION_TIME)?;
        }
        let mut die = Vec::new();
        for i in barometers.into_iter().filter(|_| on_channel) {
            let r = poll_ready(
                clock,
                || mpl3115a2::is_ready(bus),
                READY_POLL_COUNT,
                READY_POLL,
            )
            .and_then(|()| mpl3115a2::fetch(bus))
            .and_then(|(p, t)| {
                if (20_000.0..=120_000.0).contains(&p) {
                    Ok((p, t))
                } else {
                    Err(SensorError::Invalid(format!(
                        "pressure {p} Pa out of range"
                    )))
                }
            });
            if let Some((p, t)) = self.check(errors, i, r)? {
                self.measured(i);
                results[i].pressure = Some(p);
                die.push((i, t));
                // The 20-bit register: quarters of a pascal.
                self.watch_low_bits(i, (p * 4.0).round() as u32);
            }
        }
        // A Climate Module's barometer far off its thermometer: a second
        // barometer (a Barometer Tag) answers at its address too.
        for (i, t) in die {
            let thermometer = modules.iter().find_map(|m| {
                let climate = matches!(
                    m.kind,
                    ModuleKind::ClimateModuleR1 | ModuleKind::ClimateModuleR2
                );
                let mine = m.parts.iter().any(|&(_, p)| p == i);
                let t = m
                    .parts
                    .iter()
                    .find(|(metric, _)| *metric == Metric::Temperature)
                    .and_then(|&(_, p)| results[p].temperature);
                (climate && mine).then_some(t).flatten()
            });
            if let Some(reference) = thermometer {
                let part = self.parts[i];
                self.suspect(
                    part.kind,
                    part.addr,
                    (t - reference).abs() > BAROMETER_MISMATCH_C,
                );
            }
        }

        // Light sensors.
        let light = self.ready(SensorKind::Opt3001);
        if !light.is_empty() && on_channel {
            on_channel = idle_until(bus, t0 + opt3001::CONVERSION_TIME)?;
        }
        for i in light.into_iter().filter(|_| on_channel) {
            let addr = self.parts[i].addr;
            let r = poll_ready(
                clock,
                || opt3001::is_ready(bus, addr),
                READY_POLL_COUNT,
                READY_POLL,
            )
            .and_then(|()| opt3001::fetch_raw(bus, addr));
            if let Some(raw) = self.check(errors, i, r)? {
                self.measured(i);
                results[i].illuminance = Some(opt3001::lux(raw));
                // The mantissa; in the dark it is too small to tell.
                let mantissa = raw & 0x0FFF;
                if mantissa >= 64 {
                    self.watch_low_bits(i, u32::from(mantissa));
                }
            }
        }

        // Each value goes to its source's series: the channel's reading for
        // the primary sources, the sensor's own for the others.
        let mut values = Values::default();
        let mut extra: Vec<(String, Values)> = Vec::new();
        for module in &modules {
            for &(metric, i) in &module.parts {
                let Some(v) = results[i].get(metric) else {
                    continue;
                };
                let id = self.parts[i].id();
                let primary = sources
                    .iter()
                    .any(|s| s.metric == metric && s.primary && s.id == id);
                if primary {
                    values.set(metric, Some(v));
                } else if let Some((_, e)) = extra.iter_mut().find(|(x, _)| *x == id) {
                    e.set(metric, Some(v));
                } else {
                    let mut e = Values::default();
                    e.set(metric, Some(v));
                    extra.push((id, e));
                }
            }
        }

        // CO2 last, when due: the LP8 takes about 0.7 s, its waits lent out.
        if let Some(co2) = self.co2.as_mut()
            && on_channel
            && co2.due(clock.now())
        {
            match co2.measure(bus, clock, &mut *idle, values.pressure.or(pressure_hint)) {
                Ok(_) => {}
                Err(co2::Co2Error::Bus(e)) if e.is_fatal() => return Err(e),
                Err(e) => warn!("CO2 Module on channel {channel}: {e}"),
            }
            if co2.gone() {
                info!("CO2 Module on channel {channel} stopped answering");
                self.co2 = None;
            }
        }
        let now = clock.now();
        values.co2 = self.co2.as_ref().and_then(|c| c.value(now, CO2_MAX_AGE));
        values.voc = self.voc.as_ref().and_then(|v| v.value(now, VOC_MAX_AGE));

        self.parts.retain(|p| p.present());
        self.tidy_watched(now);
        self.note_r1();
        report.values = values;
        report.extra = extra;
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
    /// How the Climate Module R1 parts are named, per channel.
    r1: [ClimateR1; 2],
    /// Channels with at least one part, in channel order.
    channels: Vec<ChannelSensors>,
    state_dir: Option<PathBuf>,
    /// The latest values of each channel.
    last: Vec<(u8, Values)>,
    /// CO2 and VOC modules of channels forgotten by [`forget`](Self::forget).
    spares: Vec<ChannelSensors>,
}

impl Sensors {
    pub fn new(mode: ChannelMode) -> Self {
        Self {
            mode,
            r1: [ClimateR1::Auto; 2],
            channels: Vec::new(),
            state_dir: None,
            last: Vec::new(),
            spares: Vec::new(),
        }
    }

    /// Where the LP8 and SGPC3 keep their state between runs.
    pub fn with_state_dir(mut self, dir: Option<PathBuf>) -> Self {
        self.state_dir = dir;
        self
    }

    fn last(&self, channel: u8) -> Option<&Values> {
        self.last
            .iter()
            .find(|(c, _)| *c == channel)
            .map(|(_, v)| v)
    }

    /// Switch the CO2 Module's sensor and charging off (the app stops).
    pub fn stop(&self, bus: &mut dyn I2cBus) {
        for co2 in self.channels.iter().filter_map(|c| c.co2.as_ref()) {
            let _ = co2.stop(bus);
        }
    }

    /// When [`service`](Self::service) has work next (clock time).
    pub fn next_service(&self) -> Option<Duration> {
        self.channels
            .iter()
            .filter_map(|c| c.voc.as_ref().map(Sgpc3::next))
            .min()
    }

    /// Measure TVOC on the channels where it is due (between cycles).
    pub fn service(&mut self, bus: &mut dyn I2cBus, clock: &dyn Clock) -> Result<(), BusError> {
        let now = clock.now();
        for i in 0..self.channels.len() {
            if !self.channels[i].voc.as_ref().is_some_and(|v| v.due(now)) {
                continue;
            }
            let channel = self.channels[i].channel;
            let climate = self
                .last(channel)
                .and_then(|v| v.temperature.zip(v.humidity));
            match select_channel(bus, channel) {
                Ok(()) => self.channels[i].service(bus, clock, climate)?,
                Err(e) if e.is_fatal() => return Err(e),
                Err(e) => {
                    // The slot is lost; never leave the job due, or the
                    // caller would serve it again at once, forever.
                    debug!("I2C switch did not accept channel {channel}: {e}");
                    self.channels[i].missed(now);
                }
            }
        }
        self.channels.retain(|c| !c.is_empty());
        Ok(())
    }

    /// How the Climate Module R1 parts are named on each channel; takes
    /// effect at once (the sources change, the parts stay).
    pub fn set_climate_r1(&mut self, r1: [ClimateR1; 2]) {
        self.r1 = r1;
        for c in self.channels.iter_mut().chain(self.spares.iter_mut()) {
            c.r1 = r1_of(r1, c.channel);
        }
    }

    pub fn with_climate_r1(mut self, r1: [ClimateR1; 2]) -> Self {
        self.set_climate_r1(r1);
        self
    }

    pub fn set_mode(&mut self, mode: ChannelMode) {
        self.mode = mode;
        self.forget();
    }

    /// Forget every part found; the next [`detect`](Self::detect) starts
    /// over. Running CO2 and VOC modules are kept aside and taken over when
    /// they are found again: they kept their power on the same bridge.
    pub fn forget(&mut self) {
        self.spares = self
            .channels
            .drain(..)
            .filter(|c| c.co2.is_some() || c.voc.is_some())
            .map(|c| {
                let mut spare = ChannelSensors::new(c.channel, c.state_dir.clone(), c.r1);
                spare.spare_co2 = c.co2;
                spare.spare_voc = c.voc;
                spare
            })
            .collect();
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

    /// Parts whose data suggest a second device at their address, by
    /// channel.
    pub fn warnings(&self) -> Vec<BusWarning> {
        self.channels.iter().flat_map(|c| c.warnings()).collect()
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
                None => match self.spares.iter().position(|s| s.channel == channel) {
                    Some(i) => self.spares.remove(i),
                    None => ChannelSensors::new(
                        channel,
                        self.state_dir.clone(),
                        r1_of(self.r1, channel),
                    ),
                },
            };
            if !sensors.probe(bus, clock)? {
                continue;
            }
            found = true;
            info!(
                "I2C channel {channel}: {}",
                describe_parts(&sensors.sources())
            );
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
        for i in 0..self.channels.len() {
            // Pressure is the same throughout a building: any barometer.
            let pressure = self.last.iter().find_map(|(_, v)| v.pressure);
            let sensors = &mut self.channels[i];
            let report = sensors.measure_with(bus, clock, idle, pressure)?;
            let channel = sensors.channel;
            self.last.retain(|(c, _)| *c != channel);
            self.last.push((channel, report.values));
            reports.push((channel, report));
        }
        self.channels.retain(|c| !c.is_empty());
        // A channel no longer measured must not lend its pressure to the LP8.
        let channels = &self.channels;
        self.last
            .retain(|(c, _)| channels.iter().any(|ch| ch.channel == *c));
        Ok(reports)
    }
}

fn r1_of(r1: [ClimateR1; 2], channel: u8) -> ClimateR1 {
    r1.get(usize::from(channel)).copied().unwrap_or_default()
}

/// Every part once, for the log: `Climate Module R2 SHT30 0x45, Humidity
/// Tag SHT20 0x40`.
fn describe_parts(sources: &[SensorSource]) -> String {
    let mut seen: Vec<&str> = Vec::new();
    let mut names = Vec::new();
    for s in sources {
        if !seen.contains(&s.id.as_str()) {
            seen.push(&s.id);
            names.push(format!(
                "{} {} 0x{:02X}",
                s.module.name(),
                s.sensor,
                s.address
            ));
        }
    }
    names.join(", ")
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
        MockBus, MockClock, SharedEnv, SimMpl3115a2, SimOpt3001, SimSht20, SimSht30, SimTmp112,
        Transaction,
    };
    use crate::model::ClimateRevision;

    fn env() -> SharedEnv {
        Arc::new(Mutex::new(Values {
            temperature: Some(23.5),
            humidity: Some(41.0),
            illuminance: Some(512.0),
            pressure: Some(98_765.25),
            co2: None,
            voc: None,
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

    fn tag(t: f64, rh: f64) -> SharedEnv {
        Arc::new(Mutex::new(Values {
            temperature: Some(t),
            humidity: Some(rh),
            ..Values::default()
        }))
    }

    /// The temperature sources: module, sensor id and whether primary.
    fn temperatures(sensors: &Sensors) -> Vec<(ModuleKind, String, bool)> {
        sensors
            .sources()
            .into_iter()
            .filter(|s| s.metric == Metric::Temperature)
            .map(|s| (s.module, s.id, s.primary))
            .collect()
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
            crate::model::source_label(&sources, Metric::Humidity, 1, None, "channel {n}")
                .as_deref(),
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
            co2: None,
            voc: None,
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

    /// An HDC2080 (an older Humidity Tag): registers behind a pointer,
    /// zeros apart from its manufacturer ID.
    struct SimHdc2080 {
        pointer: u8,
    }

    impl crate::bus::mock::SimDevice for SimHdc2080 {
        fn write(&mut self, data: &[u8]) -> Result<(), BusError> {
            self.pointer = *data.first().ok_or(BusError::DataNack)?;
            Ok(())
        }

        fn read(&mut self, buf: &mut [u8]) -> Result<(), BusError> {
            for (i, b) in buf.iter_mut().enumerate() {
                *b = match self.pointer.wrapping_add(i as u8) {
                    0xFC => 0x49,
                    0xFD => 0x54,
                    _ => 0,
                };
            }
            Ok(())
        }
    }

    #[test]
    fn an_hdc2080_is_not_taken_for_an_sht20() -> Result<(), BusError> {
        let mut bus = MockBus::new();
        bus.attach(0, 0x40, Box::new(SimHdc2080 { pointer: 0 }));
        let clock = MockClock::new();
        let mut sensors = Sensors::new(ChannelMode::Channel0);
        sensors.detect(&mut bus, &clock)?;
        assert!(sensors.is_empty(), "{:?}", sensors.sources());
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
        let mut bus = MockBus::with_climate(ClimateRevision::R1, 0, env());
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
    fn a_temperature_tag_at_0x49_next_to_a_climate_module_r1() -> Result<(), BusError> {
        let mut bus = MockBus::with_climate(ClimateRevision::R1, 0, env());
        bus.attach(0, 0x49, Box::new(SimTmp112::new(tag(5.5, 80.0))));
        let clock = MockClock::new();
        let mut sensors = Sensors::new(ChannelMode::Channel0);
        sensors.detect(&mut bus, &clock)?;
        assert_eq!(
            temperatures(&sensors),
            vec![
                (ModuleKind::ClimateModuleR1, "tmp112-48".into(), true),
                (ModuleKind::TemperatureTag, "tmp112-49".into(), false),
            ],
            "the module's TMP112 is the channel's, the tag's has its own series"
        );
        let report = single(sensors.measure(&mut bus, &clock)?);
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert_close(report.values.temperature, 23.5, 0.07);
        assert_eq!(report.extra.len(), 1, "{:?}", report.extra);
        assert_eq!(report.extra[0].0, "tmp112-49");
        assert_close(report.extra[0].1.temperature, 5.5, 0.07);
        Ok(())
    }

    /// The setup of the test of 2026-11-16 (GitHub issue #2): a Climate
    /// Module R2 with a Humidity Tag and a Temperature Tag on channel 0.
    #[test]
    fn tags_next_to_a_climate_module_r2_get_their_own_series() -> Result<(), BusError> {
        let mut bus = MockBus::with_climate(ClimateRevision::R2, 0, env());
        bus.attach(0, 0x40, Box::new(SimSht20::new(tag(5.5, 80.0))));
        bus.attach(0, 0x48, Box::new(SimTmp112::new(tag(-3.25, 0.0))));
        let clock = MockClock::new();
        let mut sensors = Sensors::new(ChannelMode::Channel0);
        sensors.detect(&mut bus, &clock)?;
        assert_eq!(
            modules(&sensors),
            vec![
                (0, ModuleKind::ClimateModuleR2),
                (0, ModuleKind::HumidityTag),
                (0, ModuleKind::TemperatureTag),
            ],
            "not a Climate Module R1: the SHT20 and TMP112 sit next to an R2"
        );
        assert_eq!(
            temperatures(&sensors),
            vec![
                (ModuleKind::ClimateModuleR2, "sht30-45".into(), true),
                (ModuleKind::TemperatureTag, "tmp112-48".into(), false),
                (ModuleKind::HumidityTag, "sht20-40".into(), false),
            ]
        );
        let humidity: Vec<(String, bool)> = sensors
            .sources()
            .into_iter()
            .filter(|s| s.metric == Metric::Humidity)
            .map(|s| (s.id, s.primary))
            .collect();
        assert_eq!(
            humidity,
            vec![("sht30-45".into(), true), ("sht20-40".into(), false)]
        );
        let report = single(sensors.measure(&mut bus, &clock)?);
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert_close(report.values.temperature, 23.5, 0.01);
        assert_close(report.values.humidity, 41.0, 0.01);
        assert_close(report.values.illuminance, 512.0, 0.1);
        let extra = |id: &str| {
            report
                .extra
                .iter()
                .find(|(x, _)| x == id)
                .map(|(_, v)| *v)
                .unwrap_or_default()
        };
        assert_eq!(report.extra.len(), 2, "{:?}", report.extra);
        assert_close(extra("sht20-40").temperature, 5.5, 0.05);
        assert_close(extra("sht20-40").humidity, 80.0, 0.1);
        assert_close(extra("tmp112-48").temperature, -3.25, 0.07);
        assert_eq!(extra("tmp112-48").humidity, None);
        Ok(())
    }

    #[test]
    fn two_lux_meter_tags_at_0x44_and_0x45() -> Result<(), BusError> {
        let mut bus = MockBus::new();
        bus.attach(0, 0x44, Box::new(SimOpt3001::new(env())));
        bus.attach(
            0,
            0x45,
            Box::new(SimOpt3001::new(Arc::new(Mutex::new(Values {
                illuminance: Some(9000.0),
                ..Values::default()
            })))),
        );
        let clock = MockClock::new();
        let mut sensors = Sensors::new(ChannelMode::Channel0);
        sensors.detect(&mut bus, &clock)?;
        let ids: Vec<(String, bool)> = sensors
            .sources()
            .into_iter()
            .map(|s| (s.id, s.primary))
            .collect();
        assert_eq!(
            ids,
            vec![("opt3001-44".into(), true), ("opt3001-45".into(), false)]
        );
        let report = single(sensors.measure(&mut bus, &clock)?);
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert_close(report.values.illuminance, 512.0, 0.5);
        assert_eq!(report.extra.len(), 1);
        assert_close(report.extra[0].1.illuminance, 9000.0, 10.0);
        Ok(())
    }

    #[test]
    fn a_humidity_tag_r4_with_an_sht30_at_0x44() -> Result<(), BusError> {
        let mut bus = MockBus::new();
        bus.attach(1, 0x44, Box::new(SimSht30::new(tag(7.5, 66.0))));
        let clock = MockClock::new();
        let mut sensors = Sensors::new(ChannelMode::Auto);
        sensors.detect(&mut bus, &clock)?;
        let sources = sensors.sources();
        assert_eq!(sources.len(), 2, "{sources:?}");
        assert!(sources.iter().all(|s| s.module == ModuleKind::HumidityTag
            && s.sensor == "SHT30"
            && s.address == 0x44
            && s.primary));
        let report = single(sensors.measure(&mut bus, &clock)?);
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert_close(report.values.temperature, 7.5, 0.01);
        assert_close(report.values.humidity, 66.0, 0.01);
        Ok(())
    }

    #[test]
    fn tags_at_the_climate_module_r1_addresses_are_tags() -> Result<(), BusError> {
        let mut bus = MockBus::new();
        bus.attach(0, 0x40, Box::new(SimSht20::new(tag(5.5, 80.0))));
        bus.attach(0, 0x48, Box::new(SimTmp112::new(env())));
        let clock = MockClock::new();
        let mut sensors = Sensors::new(ChannelMode::Channel0);
        sensors.detect(&mut bus, &clock)?;
        assert_eq!(
            modules(&sensors),
            vec![
                (0, ModuleKind::HumidityTag),
                (0, ModuleKind::TemperatureTag)
            ],
            "a Climate Module R1 always has its light sensor and barometer too"
        );
        assert_eq!(
            temperatures(&sensors),
            vec![
                (ModuleKind::TemperatureTag, "tmp112-48".into(), true),
                (ModuleKind::HumidityTag, "sht20-40".into(), false),
            ]
        );
        let report = single(sensors.measure(&mut bus, &clock)?);
        assert_close(report.values.temperature, 23.5, 0.07);
        assert_close(report.values.humidity, 80.0, 0.1);
        assert_close(report.extra[0].1.temperature, 5.5, 0.05);
        Ok(())
    }

    #[test]
    fn all_four_r1_parts_are_a_climate_module_r1_unless_set_to_tags() -> Result<(), BusError> {
        let mut bus = MockBus::with_climate(ClimateRevision::R1, 0, env());
        let clock = MockClock::new();
        let mut sensors = Sensors::new(ChannelMode::Channel0);
        sensors.detect(&mut bus, &clock)?;
        assert_eq!(modules(&sensors), vec![(0, ModuleKind::ClimateModuleR1)]);
        sensors.set_climate_r1([ClimateR1::Tags, ClimateR1::Auto]);
        assert_eq!(
            modules(&sensors),
            vec![
                (0, ModuleKind::HumidityTag),
                (0, ModuleKind::TemperatureTag),
                (0, ModuleKind::LuxMeterTag),
                (0, ModuleKind::BarometerTag),
            ]
        );
        let ids: Vec<String> = sensors.sources().into_iter().map(|s| s.id).collect();
        assert!(
            ids.contains(&"tmp112-48".to_string()) && ids.contains(&"sht20-40".to_string()),
            "the parts keep their ids, so their series go on: {ids:?}"
        );
        // Only the thermometers: a module only when the user says so.
        let mut bus = MockBus::new();
        bus.attach(0, 0x40, Box::new(SimSht20::new(env())));
        bus.attach(0, 0x48, Box::new(SimTmp112::new(env())));
        let mut sensors =
            Sensors::new(ChannelMode::Channel0).with_climate_r1([ClimateR1::Module; 2]);
        sensors.detect(&mut bus, &clock)?;
        assert_eq!(modules(&sensors), vec![(0, ModuleKind::ClimateModuleR1)]);
        Ok(())
    }

    /// GitHub issue #2: a Humidity Tag next to a Climate Module R1, both
    /// SHT20 at 0x40, garble each other's answers.
    #[test]
    fn two_sht20_at_one_address_are_reported() -> Result<(), BusError> {
        use crate::bus::mock::SimCollision;
        let mut bus = MockBus::with_climate(ClimateRevision::R1, 0, env());
        bus.attach(
            0,
            0x40,
            Box::new(SimCollision {
                a: Box::new(SimSht20::new(env())),
                b: Box::new(SimSht20::new(tag(5.5, 80.0))),
            }),
        );
        let clock = MockClock::new();
        let mut sensors = Sensors::new(ChannelMode::Channel0);
        sensors.detect(&mut bus, &clock)?;
        assert!(sensors.warnings().is_empty(), "nothing known yet");
        for _ in 0..3 {
            sensors.measure(&mut bus, &clock)?;
            sensors.detect(&mut bus, &clock)?;
        }
        let warnings = sensors.warnings();
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(
            (
                warnings[0].channel,
                warnings[0].address,
                warnings[0].sensor.as_str()
            ),
            (0, 0x40, "SHT20")
        );
        // The garbled part is given up and found again by the probe, once
        // a minute; at a 2 s interval the warning used to go in between.
        for cycle in 0..150 {
            sensors.measure(&mut bus, &clock)?;
            clock.sleep(Duration::from_secs(1));
            if cycle % 30 == 29 {
                sensors.detect(&mut bus, &clock)?;
            }
            assert_eq!(sensors.warnings().len(), 1, "cycle {cycle}");
        }
        // The tag is taken out: the module's own SHT20 reads clean, and the
        // warning ends a few cycles later.
        bus.attach(0, 0x40, Box::new(SimSht20::new(env())));
        sensors.detect(&mut bus, &clock)?;
        for _ in 0..=SUSPECT_MAX {
            sensors.measure(&mut bus, &clock)?;
        }
        assert!(sensors.warnings().is_empty(), "{:?}", sensors.warnings());
        Ok(())
    }

    /// A Barometer Tag next to a Climate Module R2, both MPL3115A2 at 0x60:
    /// the barometer's die reads far off the module's thermometer.
    #[test]
    fn two_barometers_at_one_address_are_reported() -> Result<(), BusError> {
        use crate::bus::mock::SimCollision;
        let mut bus = MockBus::with_climate(ClimateRevision::R2, 0, env());
        let outdoor: SharedEnv = Arc::new(Mutex::new(Values {
            temperature: Some(-8.0),
            pressure: Some(99_000.0),
            ..Values::default()
        }));
        bus.attach(
            0,
            0x60,
            Box::new(SimCollision {
                a: Box::new(SimMpl3115a2::new(env())),
                b: Box::new(SimMpl3115a2::new(outdoor)),
            }),
        );
        let clock = MockClock::new();
        let mut sensors = Sensors::new(ChannelMode::Channel0);
        sensors.detect(&mut bus, &clock)?;
        for _ in 0..4 {
            sensors.measure(&mut bus, &clock)?;
        }
        let warnings = sensors.warnings();
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(
            (warnings[0].address, warnings[0].module),
            (0x60, ModuleKind::ClimateModuleR2)
        );
        Ok(())
    }

    /// A small xorshift, so the tests draw the same values every run.
    struct Noise(u64);

    impl Noise {
        fn next(&mut self) -> u32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 16) as u32
        }
    }

    /// Two sensors at one address, each wobbling over `span` codes, read
    /// ANDed.
    fn anded(noise: &mut Noise, span: u32) -> u32 {
        (390_000 + noise.next() % span) & (390_010 + noise.next() % span)
    }

    #[test]
    fn low_bits_tell_two_ands_from_one_sensor() {
        let mut noise = Noise(0x9E37_79B9_7F4A_7C15);
        // A lone sensor drifting and wobbling: never suspect.
        let mut lone = LowBits::default();
        for i in 0..2000u32 {
            let raw = 390_000 + i / 4 + noise.next() % 24;
            assert!(!lone.push(raw), "a lone sensor was suspected at {i}");
        }
        // A steady reading flipping between two values, one of them a
        // multiple of eight: nothing gathered at all.
        let mut steady = LowBits::default();
        for i in 0..500u32 {
            assert!(!steady.push(392 + i % 2));
            assert_eq!(steady.score, 0);
        }
        // Two sensors read ANDed: suspect after about 150 changed readings,
        // and for good.
        let mut pair = LowBits::default();
        let mut since = None;
        for i in 0..3000u32 {
            let suspect = pair.push(anded(&mut noise, 64));
            if suspect && since.is_none() {
                since = Some(i);
            }
            assert!(since.is_none() || suspect, "dropped out at {i}");
        }
        assert!(since.is_some_and(|i| (60..300).contains(&i)), "{since:?}");
    }

    /// The earlier detector (22 zeros among the last 90 changed readings)
    /// warned about a lone sensor several times a month, and five times in
    /// 70 minutes on the real set-up: a month of readings at the default
    /// interval and more, with several series of noise, must stay quiet.
    #[test]
    fn low_bits_of_a_lone_sensor_stay_quiet_for_months() {
        // 130 000 readings: 30 days at the default 20 s.
        for seed in 1..=4u64 {
            let mut noise = Noise(0x9E37_79B9_7F4A_7C15 ^ (seed * 0x0123_4567_89AB_CDEF));
            // Its lowest bits at random, on a slow drift.
            let mut drifting = LowBits::default();
            // The worst case that still varies: spread evenly over 17
            // codes that start at a multiple of eight (3 in 17).
            let mut aligned = LowBits::default();
            let mut highest = 0;
            for i in 0..130_000u32 {
                let wobble = noise.next();
                assert!(
                    !drifting.push(390_000 + i / 50 + wobble % 64),
                    "seed {seed}"
                );
                assert!(!aligned.push(392_000 + (wobble >> 8) % 17), "seed {seed}");
                highest = highest.max(drifting.score).max(aligned.score);
            }
            assert!(
                highest < LOW_BITS_SUSPECT - 12,
                "seed {seed}: too close, {highest}"
            );
        }
    }

    #[test]
    fn low_bits_do_not_take_a_reading_that_settles_for_a_pair() {
        // A ramp (variety), then a flip between two values, one of them a
        // multiple of eight: it counts only while the ramp is remembered.
        let mut bits = LowBits::default();
        for round in 0..50u32 {
            for i in 0..40 {
                assert!(!bits.push(392_000 + round * 100 + i));
            }
            for i in 0..400 {
                assert!(
                    !bits.push(392_000 + round * 100 + 40 + i % 2),
                    "round {round}"
                );
            }
            assert_eq!(bits.score, 0, "faded while steady");
        }
    }

    #[test]
    fn low_bits_clear_a_part_left_alone() {
        let mut noise = Noise(0xD1B5_4A32_D192_ED03);
        let mut bits = LowBits::default();
        for _ in 0..600 {
            bits.push(anded(&mut noise, 64));
        }
        assert!(bits.suspect);
        // The second device is gone: cleared within about 65 readings.
        let cleared = (0..200u32).find(|_| !bits.push(390_000 + noise.next() % 64));
        assert!(
            cleared.is_some_and(|i| (20..100).contains(&i)),
            "{cleared:?}"
        );
        // A steady reading (no variety) ends a warning too.
        let mut bits = LowBits::default();
        for _ in 0..600 {
            bits.push(anded(&mut noise, 64));
        }
        let cleared = (0..200u32).find(|i| !bits.push(390_000 + i % 2));
        assert!(cleared.is_some(), "a steady reading kept the warning");
    }

    /// A pair is told by its changed readings alone: a cycle that reads the
    /// same value again must not take anything away (it used to, so a pair
    /// whose reading changed every third cycle was never reported).
    #[test]
    fn a_pair_whose_reading_often_repeats_is_reported() {
        let mut noise = Noise(0x9E37_79B9_7F4A_7C15);
        let mut c = ChannelSensors::new(0, None, ClimateR1::Auto);
        c.add(SensorKind::Opt3001, Slot::Ready, 0x44);
        let clock = MockClock::new();
        for _ in 0..400 {
            let raw = anded(&mut noise, 64) & 0x0FFF;
            for _ in 0..3 {
                c.measured(0);
                c.watch_low_bits(0, raw);
                c.tidy_watched(clock.now());
            }
        }
        let warnings = c.warnings();
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(warnings[0].address, 0x44);
        // The part is unplugged: what its lowest bits told goes with it.
        c.parts.clear();
        c.tidy_watched(clock.now());
        assert!(c.warnings().is_empty());
    }

    #[test]
    fn a_checksum_warning_does_not_flicker() {
        let mut c = ChannelSensors::new(0, None, ClimateR1::Auto);
        c.add(SensorKind::Sht20, Slot::Ready, 0x40);
        c.suspect(SensorKind::Sht20, 0x40, true);
        assert!(c.warnings().is_empty(), "one garbled cycle is not enough");
        c.suspect(SensorKind::Sht20, 0x40, true);
        assert_eq!(c.warnings().len(), 1);
        // Reported until the score is back at zero, not just below the
        // level that raised it.
        for _ in 0..SUSPECT_AT - 1 {
            c.suspect(SensorKind::Sht20, 0x40, false);
            assert_eq!(c.warnings().len(), 1);
        }
        c.suspect(SensorKind::Sht20, 0x40, false);
        assert!(c.warnings().is_empty());
    }

    /// GitHub issue #2: a Lux Meter Tag next to a Climate Module, both
    /// OPT3001 at 0x44. Their light differs a little from cycle to cycle.
    #[test]
    fn two_light_sensors_at_one_address_are_reported() -> Result<(), BusError> {
        use crate::bus::mock::SimCollision;
        let module = env();
        let tag: SharedEnv = Arc::new(Mutex::new(Values::default()));
        let mut bus = MockBus::with_climate(ClimateRevision::R2, 0, module.clone());
        bus.attach(
            0,
            0x44,
            Box::new(SimCollision {
                a: Box::new(SimOpt3001::new(module.clone())),
                b: Box::new(SimOpt3001::new(tag.clone())),
            }),
        );
        let clock = MockClock::new();
        let mut sensors = Sensors::new(ChannelMode::Channel0);
        sensors.detect(&mut bus, &clock)?;
        let mut noise = Noise(0x2545_F491_4F6C_DD1D);
        for _ in 0..400 {
            if let (Ok(mut m), Ok(mut t)) = (module.lock(), tag.lock()) {
                m.illuminance = Some(400.0 + f64::from(noise.next() % 6000) / 100.0);
                t.illuminance = Some(420.0 + f64::from(noise.next() % 6000) / 100.0);
            }
            sensors.measure(&mut bus, &clock)?;
        }
        let warnings = sensors.warnings();
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(
            (warnings[0].address, warnings[0].sensor.as_str()),
            (0x44, "OPT3001")
        );
        Ok(())
    }

    #[test]
    fn a_lone_climate_module_with_changing_values_raises_no_warning() -> Result<(), BusError> {
        let module = env();
        let mut bus = MockBus::with_climate(ClimateRevision::R2, 0, module.clone());
        bus.attach(0, 0x48, Box::new(SimTmp112::new(module.clone())));
        let clock = MockClock::new();
        let mut sensors = Sensors::new(ChannelMode::Channel0);
        sensors.detect(&mut bus, &clock)?;
        let mut noise = Noise(0xD1B5_4A32_D192_ED03);
        for i in 0..400u32 {
            if let Ok(mut m) = module.lock() {
                m.temperature =
                    Some(22.0 + f64::from(i) / 100.0 + f64::from(noise.next() % 50) / 100.0);
                m.illuminance = Some(300.0 + f64::from(noise.next() % 30_000) / 100.0);
                m.pressure = Some(97_700.0 + f64::from(noise.next() % 400) / 4.0);
            }
            sensors.measure(&mut bus, &clock)?;
            assert!(
                sensors.warnings().is_empty(),
                "cycle {i}: {:?}",
                sensors.warnings()
            );
        }
        Ok(())
    }

    #[test]
    fn a_lone_climate_module_raises_no_warning() -> Result<(), BusError> {
        for revision in [ClimateRevision::R1, ClimateRevision::R2] {
            let mut bus = MockBus::with_climate(revision, 0, env());
            let clock = MockClock::new();
            let mut sensors = Sensors::new(ChannelMode::Channel0);
            sensors.detect(&mut bus, &clock)?;
            for _ in 0..5 {
                sensors.measure(&mut bus, &clock)?;
            }
            assert!(sensors.warnings().is_empty(), "{:?}", sensors.warnings());
        }
        Ok(())
    }

    #[test]
    fn a_climate_module_r1_stays_one_when_its_light_sensor_stops() -> Result<(), BusError> {
        let mut bus = MockBus::with_climate(ClimateRevision::R1, 0, env());
        let clock = MockClock::new();
        let mut sensors = Sensors::new(ChannelMode::Channel0);
        sensors.detect(&mut bus, &clock)?;
        bus.attach(0, 0x44, Box::new(Mute));
        for _ in 0..PART_GIVE_UP {
            sensors.measure(&mut bus, &clock)?;
        }
        assert_eq!(
            sensors.sources().len(),
            3,
            "the light sensor is gone: {:?}",
            sensors.sources()
        );
        assert_eq!(modules(&sensors), vec![(0, ModuleKind::ClimateModuleR1)]);
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

    /// Run measurement cycles every `step` of clock time until `until`.
    fn cycles(
        sensors: &mut Sensors,
        bus: &mut MockBus,
        clock: &MockClock,
        step: Duration,
        until: Duration,
    ) -> Result<Vec<Values>, BusError> {
        let mut out = Vec::new();
        while clock.now() < until {
            for (_, report) in sensors.measure(bus, clock)? {
                out.push(report.values);
            }
            sensors.service(bus, clock)?;
            clock.sleep(step);
        }
        Ok(out)
    }

    #[test]
    fn co2_after_the_first_charge_with_the_state_handed_back() -> Result<(), BusError> {
        let mut bus = MockBus::with_climate(ClimateRevision::R2, 0, env());
        let lp8 = crate::bus::mock::attach_co2_module(&mut bus, 0);
        let clock = MockClock::new();
        let mut sensors = Sensors::new(ChannelMode::Auto);
        sensors.detect(&mut bus, &clock)?;
        assert!(
            modules(&sensors).contains(&(0, ModuleKind::Co2Module)),
            "{:?}",
            modules(&sensors)
        );
        // Nothing while the capacitor charges for the first minute.
        let early = cycles(
            &mut sensors,
            &mut bus,
            &clock,
            Duration::from_secs(20),
            Duration::from_secs(50),
        )?;
        assert!(early.iter().all(|v| v.co2.is_none()));
        let later = cycles(
            &mut sensors,
            &mut bus,
            &clock,
            Duration::from_secs(20),
            Duration::from_secs(130),
        )?;
        assert_eq!(later.last().and_then(|v| v.co2), Some(650.0));
        let writes = lp8.lock().map(|l| l.writes.clone()).unwrap_or_default();
        assert!(writes.len() >= 3, "{writes:?}");
        // Initial first, then sequential with the state the last reply gave.
        assert_eq!(writes[0].0, 0x10);
        assert!(writes[1..].iter().all(|w| w.0 == 0x20));
        assert_eq!(writes[2].1[0], writes[1].1[0].wrapping_add(1));
        // The barometer on the channel compensates: 98 765 Pa in 0.1 hPa.
        assert_eq!(writes[1].2, 9877);
        Ok(())
    }

    #[test]
    fn co2_state_survives_a_restart_and_a_fault_starts_over() -> Result<(), BusError> {
        let Ok(dir) = tempfile::tempdir() else {
            panic!("no temp dir");
        };
        let mut bus = MockBus::new();
        let lp8 = crate::bus::mock::attach_co2_module(&mut bus, 1);
        let clock = MockClock::new();
        let mut sensors = Sensors::new(ChannelMode::Auto).with_state_dir(Some(dir.path().into()));
        sensors.detect(&mut bus, &clock)?;
        cycles(
            &mut sensors,
            &mut bus,
            &clock,
            Duration::from_secs(20),
            Duration::from_secs(90),
        )?;
        assert!(dir.path().join("lp8-channel1.txt").exists());

        // A new run (the app restarted) continues from the stored state.
        let mut sensors = Sensors::new(ChannelMode::Auto).with_state_dir(Some(dir.path().into()));
        sensors.detect(&mut bus, &clock)?;
        let before = lp8.lock().map(|l| l.writes.len()).unwrap_or(0);
        let until = clock.now() + Duration::from_secs(90);
        cycles(
            &mut sensors,
            &mut bus,
            &clock,
            Duration::from_secs(20),
            until,
        )?;
        let writes = lp8.lock().map(|l| l.writes.clone()).unwrap_or_default();
        assert_eq!(writes[before].0, 0x20, "sequential after the restart");

        // A fault the datasheet answers with a power cycle: initial again,
        // and no value from the faulty measurement.
        if let Ok(mut l) = lp8.lock() {
            l.status = [crate::sensors::lp8::status0::ALGORITHM, 0];
        }
        let n = writes.len();
        let until = clock.now() + Duration::from_secs(20);
        cycles(
            &mut sensors,
            &mut bus,
            &clock,
            Duration::from_secs(20),
            until,
        )?;
        if let Ok(mut l) = lp8.lock() {
            l.status = [0, 0];
        }
        let until = clock.now() + Duration::from_secs(40);
        cycles(
            &mut sensors,
            &mut bus,
            &clock,
            Duration::from_secs(20),
            until,
        )?;
        let writes = lp8.lock().map(|l| l.writes.clone()).unwrap_or_default();
        assert_eq!(writes[n].0, 0x20);
        assert_eq!(writes[n + 1].0, 0x10, "initial after the fault: {writes:?}");
        Ok(())
    }

    #[test]
    fn voc_in_ultra_low_power_with_humidity_from_its_channel() -> Result<(), BusError> {
        let Ok(dir) = tempfile::tempdir() else {
            panic!("no temp dir");
        };
        let mut bus = MockBus::with_climate(ClimateRevision::R2, 0, env());
        let sgp = crate::bus::mock::attach_sgpc3(&mut bus, 0);
        let clock = MockClock::new();
        let mut sensors = Sensors::new(ChannelMode::Auto).with_state_dir(Some(dir.path().into()));
        sensors.detect(&mut bus, &clock)?;
        assert!(modules(&sensors).contains(&(0, ModuleKind::VocLpTag)));
        let commands = |sgp: &Arc<Mutex<crate::bus::mock::SimSgpc3>>| {
            sgp.lock().map(|s| s.commands.clone()).unwrap_or_default()
        };
        let start = commands(&sgp);
        let pos = |cmd: [u8; 2]| start.iter().position(|c| c.0 == cmd);
        // Ultra-low power before the algorithm starts.
        assert_eq!(
            start.iter().find(|c| c.0 == [0x20, 0x9F]).map(|c| c.1),
            Some(Some(0))
        );
        assert!(pos([0x20, 0x9F]) < pos([0x20, 0xAE]));

        // Zeros while preheating are not values.
        if let Ok(mut s) = sgp.lock() {
            s.tvoc = 0;
        }
        let preheat = cycles(
            &mut sensors,
            &mut bus,
            &clock,
            Duration::from_secs(20),
            Duration::from_secs(200),
        )?;
        assert!(preheat.iter().all(|v| v.voc.is_none()), "{preheat:?}");
        if let Ok(mut s) = sgp.lock() {
            s.tvoc = 87;
        }
        let warm = cycles(
            &mut sensors,
            &mut bus,
            &clock,
            Duration::from_secs(20),
            Duration::from_secs(400),
        )?;
        assert_eq!(warm.last().and_then(|v| v.voc), Some(87.0));
        // Measured on the 30 s grid, each with the humidity of 23.5 °C, 41 %.
        let measures: Vec<_> = commands(&sgp)
            .iter()
            .filter(|c| c.0 == [0x20, 0x08])
            .cloned()
            .collect();
        assert!(
            (12..=14).contains(&measures.len()),
            "{} measurements in 400 s",
            measures.len()
        );
        let humidity = commands(&sgp)
            .iter()
            .rev()
            .find(|c| c.0 == [0x20, 0x61])
            .and_then(|c| c.1);
        assert_eq!(
            humidity,
            Some(crate::sensors::sgpc3::absolute_humidity(23.5, 41.0))
        );
        // After the 5 minute preheat a zero is a value: clean air.
        if let Ok(mut s) = sgp.lock() {
            s.tvoc = 0;
        }
        let clean = cycles(
            &mut sensors,
            &mut bus,
            &clock,
            Duration::from_secs(20),
            Duration::from_secs(460),
        )?;
        assert_eq!(clean.last().and_then(|v| v.voc), Some(0.0));

        // The baseline is stored after an hour and written back after a restart.
        cycles(
            &mut sensors,
            &mut bus,
            &clock,
            Duration::from_secs(60),
            Duration::from_secs(3700),
        )?;
        assert!(dir.path().join("sgpc3-channel0.txt").exists());
        if let Ok(mut s) = sgp.lock() {
            s.baseline = 0;
            s.commands.clear();
        }
        let mut sensors = Sensors::new(ChannelMode::Auto).with_state_dir(Some(dir.path().into()));
        sensors.detect(&mut bus, &clock)?;
        let restored = commands(&sgp)
            .iter()
            .find(|c| c.0 == [0x20, 0x1E])
            .and_then(|c| c.1);
        assert_eq!(restored, Some(0x8123));
        Ok(())
    }
}
