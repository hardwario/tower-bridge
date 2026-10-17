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
    },
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
}

/// The modules on each channel, in channel order, without repeats.
pub fn modules(sources: &[SensorSource]) -> Vec<(u8, ModuleKind)> {
    let mut out: Vec<(u8, ModuleKind)> = sources.iter().map(|s| (s.channel, s.module)).collect();
    out.sort();
    out.dedup();
    out
}

/// Which source of `metric` an alert on `channel` comes from, for a
/// notification: `None` while only one channel measures the metric, else the
/// module names (`Humidity Tag`), with the channel when both channels carry
/// the same modules (`Humidity Tag, channel 1`).
pub fn source_label(sources: &[SensorSource], metric: Metric, channel: u8) -> Option<String> {
    let names = |c: u8| -> Vec<&str> {
        let mut n: Vec<&str> = sources
            .iter()
            .filter(|s| s.metric == metric && s.channel == c)
            .map(|s| s.module.name())
            .collect();
        n.dedup();
        n
    };
    let mut channels: Vec<u8> = sources
        .iter()
        .filter(|s| s.metric == metric)
        .map(|s| s.channel)
        .collect();
    channels.sort_unstable();
    channels.dedup();
    if channels.len() < 2 {
        return None;
    }
    let mine = names(channel);
    if mine.is_empty() {
        return Some(format!("channel {channel}"));
    }
    let same = channels.iter().any(|&c| c != channel && names(c) == mine);
    let label = mine.join(", ");
    Some(if same {
        format!("{label}, channel {channel}")
    } else {
        label
    })
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
        }
    }

    #[test]
    fn source_labels_tell_channels_apart() {
        let t = Metric::Temperature;
        let one = [source(t, 0, ModuleKind::ClimateModuleR1)];
        assert_eq!(
            source_label(&one, t, 0),
            None,
            "a single source needs no label"
        );
        let two = [
            source(t, 0, ModuleKind::ClimateModuleR1),
            source(t, 1, ModuleKind::HumidityTag),
        ];
        assert_eq!(source_label(&two, t, 1).as_deref(), Some("Humidity Tag"));
        assert_eq!(
            source_label(&two, t, 0).as_deref(),
            Some("Climate Module R1")
        );
        let twins = [
            source(t, 0, ModuleKind::HumidityTag),
            source(t, 1, ModuleKind::HumidityTag),
        ];
        assert_eq!(
            source_label(&twins, t, 1).as_deref(),
            Some("Humidity Tag, channel 1")
        );
        assert_eq!(source_label(&two, t, 5).as_deref(), Some("channel 5"));
    }

    #[test]
    fn device_state_serializes_with_tag() -> Result<(), serde_json::Error> {
        let s = serde_json::to_value(DeviceState::Searching)?;
        assert_eq!(s["status"], "searching");
        Ok(())
    }
}
