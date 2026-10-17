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
}

impl Metric {
    pub const ALL: [Metric; 4] = [
        Metric::Temperature,
        Metric::Humidity,
        Metric::Illuminance,
        Metric::Pressure,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Metric::Temperature => "temperature",
            Metric::Humidity => "humidity",
            Metric::Illuminance => "illuminance",
            Metric::Pressure => "pressure",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Metric::Temperature => "Temperature",
            Metric::Humidity => "Humidity",
            Metric::Illuminance => "Illuminance",
            Metric::Pressure => "Pressure",
        }
    }
}

/// Values produced by one measurement cycle, in SI units: °C, %RH, lux, Pa.
/// `None` means the sensor is absent or the measurement failed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Values {
    pub temperature: Option<f64>,
    pub humidity: Option<f64>,
    pub illuminance: Option<f64>,
    pub pressure: Option<f64>,
}

impl Values {
    pub fn get(&self, metric: Metric) -> Option<f64> {
        match metric {
            Metric::Temperature => self.temperature,
            Metric::Humidity => self.humidity,
            Metric::Illuminance => self.illuminance,
            Metric::Pressure => self.pressure,
        }
    }

    pub fn is_empty(&self) -> bool {
        Metric::ALL.iter().all(|m| self.get(*m).is_none())
    }
}

/// A timestamped reading from a specific bridge.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Reading {
    /// Unix time in milliseconds (UTC) at the start of the measurement cycle.
    pub ts: i64,
    /// Device identity (USB serial number or a stable substitute).
    pub device: String,
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
    /// The bridge is open but no Climate Module answers on the bus.
    #[serde(rename_all = "snake_case")]
    NoModule { device: DeviceInfo },
    /// Bridge and Climate Module are working.
    #[serde(rename_all = "snake_case")]
    Ready {
        device: DeviceInfo,
        revision: ClimateRevision,
        channel: u8,
        /// Which sensor provides each value.
        sources: Vec<SensorSource>,
    },
}

/// The sensor behind one value and its I2C address (behind the switch channel).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SensorSource {
    pub metric: Metric,
    /// Sensor part, e.g. `TMP112`.
    pub sensor: String,
    /// 7-bit I2C address.
    pub address: u8,
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

    #[test]
    fn device_state_serializes_with_tag() -> Result<(), serde_json::Error> {
        let s = serde_json::to_value(DeviceState::Searching)?;
        assert_eq!(s["status"], "searching");
        Ok(())
    }
}
