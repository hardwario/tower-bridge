//! Shared data types.

use serde::{Deserialize, Serialize};

/// Standard sea-level pressure in pascals, the default altitude reference.
pub const STANDARD_SEA_LEVEL_PA: f64 = 101_325.0;

/// A measured quantity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Metric {
    Temperature,
    Humidity,
    Illuminance,
    Pressure,
    /// Carbon dioxide (TOWER CO2 Module).
    Co2,
    /// Total volatile organic compounds (TOWER VOC-LP Tag).
    Voc,
}

impl Metric {
    pub const ALL: [Metric; 6] = [
        Metric::Temperature,
        Metric::Humidity,
        Metric::Illuminance,
        Metric::Pressure,
        Metric::Co2,
        Metric::Voc,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Metric::Temperature => "temperature",
            Metric::Humidity => "humidity",
            Metric::Illuminance => "illuminance",
            Metric::Pressure => "pressure",
            Metric::Co2 => "co2",
            Metric::Voc => "voc",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Metric::Temperature => "Temperature",
            Metric::Humidity => "Humidity",
            Metric::Illuminance => "Illuminance",
            Metric::Pressure => "Pressure",
            Metric::Co2 => "CO2",
            Metric::Voc => "VOC",
        }
    }
}

/// Values produced by one measurement cycle, in SI units: °C, %RH, lux, Pa;
/// CO2 in ppm and TVOC in ppb. `None` means the sensor is absent or the
/// measurement failed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Values {
    pub temperature: Option<f64>,
    pub humidity: Option<f64>,
    pub illuminance: Option<f64>,
    pub pressure: Option<f64>,
    #[serde(default)]
    pub co2: Option<f64>,
    #[serde(default)]
    pub voc: Option<f64>,
}

impl Values {
    pub fn get(&self, metric: Metric) -> Option<f64> {
        match metric {
            Metric::Temperature => self.temperature,
            Metric::Humidity => self.humidity,
            Metric::Illuminance => self.illuminance,
            Metric::Pressure => self.pressure,
            Metric::Co2 => self.co2,
            Metric::Voc => self.voc,
        }
    }

    pub fn set(&mut self, metric: Metric, value: Option<f64>) {
        match metric {
            Metric::Temperature => self.temperature = value,
            Metric::Humidity => self.humidity = value,
            Metric::Illuminance => self.illuminance = value,
            Metric::Pressure => self.pressure = value,
            Metric::Co2 => self.co2 = value,
            Metric::Voc => self.voc = value,
        }
    }

    pub fn is_empty(&self) -> bool {
        Metric::ALL.iter().all(|m| self.get(*m).is_none())
    }
}

/// A timestamped reading from the sensors on one switch channel of a bridge.
///
/// A channel's reading carries one value per metric, from the channel's
/// primary source of each metric (see [`SensorSource::primary`]). A sensor
/// that measures a metric the channel already has (a Humidity Tag next to a
/// Climate Module) sends readings of its own, with `sensor` set to its id.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Reading {
    /// Unix time in milliseconds (UTC) at the start of the measurement cycle;
    /// the same for every channel measured in that cycle.
    pub ts: i64,
    /// Device identity (USB serial number or a stable substitute).
    pub device: String,
    /// I2C switch channel the values come from.
    #[serde(default)]
    pub channel: u8,
    /// The sensor (see [`sensor_id`]) of a further sensor's reading; absent
    /// for the channel's reading.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sensor: Option<String>,
    #[serde(flatten)]
    pub values: Values,
    /// Altitude in metres derived from pressure and the configured sea-level reference.
    pub altitude: Option<f64>,
}

/// Climate Module hardware revision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClimateRevision {
    /// TMP112 (temperature) + SHT20 (humidity).
    R1,
    /// SHT30 (temperature + humidity).
    R2,
}

impl ClimateRevision {
    pub fn as_str(self) -> &'static str {
        match self {
            ClimateRevision::R1 => "R1",
            ClimateRevision::R2 => "R2",
        }
    }
}

/// A TOWER module or tag that carries sensors.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModuleKind {
    ClimateModuleR1,
    ClimateModuleR2,
    HumidityTag,
    TemperatureTag,
    LuxMeterTag,
    BarometerTag,
    Co2Module,
    VocLpTag,
}

impl ModuleKind {
    /// Product name, e.g. `Humidity Tag`.
    pub fn name(self) -> &'static str {
        match self {
            ModuleKind::ClimateModuleR1 => "Climate Module R1",
            ModuleKind::ClimateModuleR2 => "Climate Module R2",
            ModuleKind::HumidityTag => "Humidity Tag",
            ModuleKind::TemperatureTag => "Temperature Tag",
            ModuleKind::LuxMeterTag => "Lux Meter Tag",
            ModuleKind::BarometerTag => "Barometer Tag",
            ModuleKind::Co2Module => "CO2 Module",
            ModuleKind::VocLpTag => "VOC-LP Tag",
        }
    }

    pub fn revision(self) -> Option<ClimateRevision> {
        match self {
            ModuleKind::ClimateModuleR1 => Some(ClimateRevision::R1),
            ModuleKind::ClimateModuleR2 => Some(ClimateRevision::R2),
            _ => None,
        }
    }
}

/// Which transport is used to reach the I2C bus.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Backend {
    /// FT260 driven directly over USB HID (hidapi).
    Hid,
    /// Linux kernel `hid-ft260` driver exposed through `/dev/i2c-N`.
    I2cDev,
}

/// Identity of an opened bridge.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DeviceInfo {
    pub backend: Backend,
    pub serial: String,
    pub path: String,
    /// FT260 chip version string when known.
    pub chip: Option<String>,
    /// TOWER Bridge Modules plugged in, this one included. Only this one is
    /// read, and the FT260 has no serial number to tell them apart.
    #[serde(default = "one")]
    pub bridges_connected: u32,
}

fn one() -> u32 {
    1
}

/// Why the bridge cannot be used right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceErrorKind {
    /// No TOWER Bridge Module is plugged in.
    NotFound,
    /// The device exists but the OS denied access (Linux udev rules).
    PermissionDenied,
    /// Linux: the kernel driver owns the FT260 but `i2c-dev` is not loaded.
    I2cDevNotLoaded,
    /// Another program holds the device.
    Busy,
    /// Anything else (protocol failure during initialisation, ...).
    Failed,
}

/// Connection state reported by the acquisition thread.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum DeviceState {
    /// Looking for a bridge.
    Searching,
    /// The bridge could not be opened.
    #[serde(rename_all = "snake_case")]
    Error {
        kind: DeviceErrorKind,
        message: String,
    },
    /// The bridge is open but no sensor answers on the bus.
    #[serde(rename_all = "snake_case")]
    NoModule { device: DeviceInfo },
    /// The bridge works and sensors were found.
    #[serde(rename_all = "snake_case")]
    Ready {
        device: DeviceInfo,
        /// Which sensor provides each value on each channel, ordered by
        /// channel and metric.
        sources: Vec<SensorSource>,
        /// Parts whose data suggest a second device at their address.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        warnings: Vec<BusWarning>,
    },
}

/// A part whose data suggest that a second device answers at its address on
/// the same channel (a Lux Meter Tag next to a Climate Module, both at
/// 0x44): both answer every read, and the values come out garbled. The tag
/// belongs on the other channel.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BusWarning {
    pub channel: u8,
    /// 7-bit I2C address.
    pub address: u8,
    /// Sensor part, e.g. `MPL3115A2`.
    pub sensor: String,
    /// The module the part was taken for.
    pub module: ModuleKind,
    pub kind: BusWarningKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BusWarningKind {
    /// Two devices at one address, from repeated checksum errors or values
    /// that cannot be right.
    AddressCollision,
}

/// The sensor behind one value: switch channel, module, part and I2C address.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SensorSource {
    pub metric: Metric,
    pub channel: u8,
    pub module: ModuleKind,
    /// Sensor part, e.g. `TMP112`.
    pub sensor: String,
    /// 7-bit I2C address.
    pub address: u8,
    /// Stable id of the part on its channel, e.g. `tmp112-48` (see
    /// [`sensor_id`]): it names the series of a further sensor and its MQTT
    /// topic, and does not change when the module is named differently.
    #[serde(default)]
    pub id: String,
    /// Whether this is the channel's source of the metric, the one whose
    /// values go into the channel's reading. Every other source of the same
    /// metric on the channel has a series of its own, named by `id`.
    #[serde(default = "yes")]
    pub primary: bool,
}

fn yes() -> bool {
    true
}

impl SensorSource {
    /// The series of this source's values: `None` for the channel's
    /// series, else the sensor's id.
    pub fn series(&self) -> Option<&str> {
        (!self.primary).then_some(self.id.as_str())
    }
}

/// A sensor's id on its channel: the part and its address in hex, e.g.
/// `sht20-40`. Lower case, so it fits MQTT topics and Prometheus labels.
pub fn sensor_id(part: &str, address: u8) -> String {
    format!("{}-{address:02x}", part.to_ascii_lowercase())
}

/// The modules on each channel, in channel order, without repeats.
pub fn modules(sources: &[SensorSource]) -> Vec<(u8, ModuleKind)> {
    let mut out: Vec<(u8, ModuleKind)> = sources.iter().map(|s| (s.channel, s.module)).collect();
    out.sort();
    out.dedup();
    out
}

/// Which source of `metric` an alert on `channel` comes from, for a
/// notification: `None` while only one series measures the metric, else the
/// module name (`Humidity Tag`), with the channel when another channel's
/// series has the same name (`Humidity Tag, channel 1`), or the part and
/// address when the other one is on the same channel (`Lux Meter Tag,
/// OPT3001 0x45`). `sensor` is the further sensor's id, `None` for the
/// channel's series. `channel_name` is the channel in the reader's
/// language, with `{n}` for its number.
pub fn source_label(
    sources: &[SensorSource],
    metric: Metric,
    channel: u8,
    sensor: Option<&str>,
    channel_name: &str,
) -> Option<String> {
    let channel_text = channel_name.replace("{n}", &channel.to_string());
    let of_metric: Vec<&SensorSource> = sources.iter().filter(|s| s.metric == metric).collect();
    if of_metric.len() < 2 {
        return None;
    }
    let Some(mine) = of_metric
        .iter()
        .find(|s| s.channel == channel && s.series() == sensor)
    else {
        return Some(channel_text);
    };
    let label = mine.module.name();
    let twins: Vec<&&SensorSource> = of_metric
        .iter()
        .filter(|s| s.module == mine.module && (s.channel, &s.id) != (mine.channel, &mine.id))
        .collect();
    if twins.iter().any(|s| s.channel == channel) {
        Some(format!("{label}, {} 0x{:02X}", mine.sensor, mine.address))
    } else if twins.is_empty() {
        Some(label.to_string())
    } else {
        Some(format!("{label}, {channel_text}"))
    }
}

/// Human readable list, e.g. `Climate Module R1 (channel 0), Humidity Tag,
/// Lux Meter Tag (channel 1)`.
pub fn describe_modules(sources: &[SensorSource]) -> String {
    let modules = modules(sources);
    let mut parts = Vec::new();
    let mut channels: Vec<u8> = modules.iter().map(|(c, _)| *c).collect();
    channels.dedup();
    for channel in channels {
        let names: Vec<&str> = modules
            .iter()
            .filter(|(c, _)| *c == channel)
            .map(|(_, m)| m.name())
            .collect();
        parts.push(format!("{} (channel {channel})", names.join(", ")));
    }
    parts.join(", ")
}

impl DeviceState {
    pub fn device(&self) -> Option<&DeviceInfo> {
        match self {
            DeviceState::NoModule { device } | DeviceState::Ready { device, .. } => Some(device),
            _ => None,
        }
    }

    pub fn is_ready(&self) -> bool {
        matches!(self, DeviceState::Ready { .. })
    }

    /// Suspected address collisions while ready, otherwise none.
    pub fn warnings(&self) -> &[BusWarning] {
        match self {
            DeviceState::Ready { warnings, .. } => warnings,
            _ => &[],
        }
    }

    /// Sensor sources while ready, otherwise none.
    pub fn sources(&self) -> &[SensorSource] {
        match self {
            DeviceState::Ready { sources, .. } => sources,
            _ => &[],
        }
    }
}

/// Barometric altitude (international standard atmosphere) in metres.
pub fn altitude(pressure_pa: f64, sea_level_pa: f64) -> f64 {
    44_330.77 * (1.0 - (pressure_pa / sea_level_pa).powf(0.190_263))
}

/// Convert degrees Celsius to Fahrenheit.
pub fn celsius_to_fahrenheit(c: f64) -> f64 {
    c * 9.0 / 5.0 + 32.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn altitude_at_reference_is_zero() {
        assert!(altitude(STANDARD_SEA_LEVEL_PA, STANDARD_SEA_LEVEL_PA).abs() < 1e-9);
    }

    #[test]
    fn altitude_decreases_with_pressure() {
        let h = altitude(89_874.6, STANDARD_SEA_LEVEL_PA);
        assert!((h - 1000.0).abs() < 1.0, "h = {h}");
    }

    fn source(metric: Metric, channel: u8, module: ModuleKind) -> SensorSource {
        SensorSource {
            metric,
            channel,
            module,
            sensor: "X".into(),
            address: 0,
            id: "x-00".into(),
            primary: true,
        }
    }

    #[test]
    fn source_labels_tell_channels_apart() {
        let t = Metric::Temperature;
        let one = [source(t, 0, ModuleKind::ClimateModuleR1)];
        assert_eq!(
            source_label(&one, t, 0, None, "channel {n}"),
            None,
            "a single source needs no label"
        );
        let two = [
            source(t, 0, ModuleKind::ClimateModuleR1),
            source(t, 1, ModuleKind::HumidityTag),
        ];
        assert_eq!(
            source_label(&two, t, 1, None, "channel {n}").as_deref(),
            Some("Humidity Tag")
        );
        assert_eq!(
            source_label(&two, t, 0, None, "channel {n}").as_deref(),
            Some("Climate Module R1")
        );
        let twins = [
            source(t, 0, ModuleKind::HumidityTag),
            source(t, 1, ModuleKind::HumidityTag),
        ];
        assert_eq!(
            source_label(&twins, t, 1, None, "channel {n}").as_deref(),
            Some("Humidity Tag, channel 1")
        );
        assert_eq!(
            source_label(&two, t, 5, None, "channel {n}").as_deref(),
            Some("channel 5")
        );
    }

    #[test]
    fn source_labels_tell_sensors_on_one_channel_apart() {
        let t = Metric::Temperature;
        let further = |module, sensor: &str, address| SensorSource {
            metric: t,
            channel: 0,
            module,
            sensor: sensor.into(),
            address,
            id: sensor_id(sensor, address),
            primary: false,
        };
        let sources = [
            SensorSource {
                sensor: "SHT30".into(),
                address: 0x45,
                id: "sht30-45".into(),
                ..source(t, 0, ModuleKind::ClimateModuleR2)
            },
            further(ModuleKind::HumidityTag, "SHT20", 0x40),
            further(ModuleKind::TemperatureTag, "TMP112", 0x48),
            further(ModuleKind::TemperatureTag, "TMP112", 0x49),
        ];
        assert_eq!(
            source_label(&sources, t, 0, None, "channel {n}").as_deref(),
            Some("Climate Module R2")
        );
        assert_eq!(
            source_label(&sources, t, 0, Some("sht20-40"), "channel {n}").as_deref(),
            Some("Humidity Tag")
        );
        assert_eq!(
            source_label(&sources, t, 0, Some("tmp112-49"), "channel {n}").as_deref(),
            Some("Temperature Tag, TMP112 0x49")
        );
    }

    #[test]
    fn sensor_ids_are_part_and_address() {
        assert_eq!(sensor_id("SHT20", 0x40), "sht20-40");
        assert_eq!(sensor_id("MPL3115A2", 0x60), "mpl3115a2-60");
    }

    #[test]
    fn a_reading_names_its_sensor_only_when_it_has_one() -> Result<(), serde_json::Error> {
        let mut r = Reading {
            ts: 1,
            device: "ft260".into(),
            channel: 0,
            sensor: None,
            values: Values::default(),
            altitude: None,
        };
        assert!(serde_json::to_value(&r)?.get("sensor").is_none());
        r.sensor = Some("sht20-40".into());
        assert_eq!(serde_json::to_value(&r)?["sensor"], "sht20-40");
        Ok(())
    }

    #[test]
    fn device_state_serializes_with_tag() -> Result<(), serde_json::Error> {
        let s = serde_json::to_value(DeviceState::Searching)?;
        assert_eq!(s["status"], "searching");
        Ok(())
    }
}
