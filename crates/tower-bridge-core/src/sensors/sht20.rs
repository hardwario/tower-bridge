//! Sensirion SHT20 humidity sensor (Climate Module R1).
//!
//! Uses the "no hold master" commands: the sensor NACKs its address while a
//! conversion is running, so the driver waits a fixed time before reading.

use std::time::Duration;

use super::SensorError;
use super::crc::crc8;
use crate::bus::{BusError, I2cBus};

pub const ADDR: u8 = 0x40;
pub const CMD_SOFT_RESET: u8 = 0xFE;
pub const CMD_TEMPERATURE: u8 = 0xF3;
pub const CMD_HUMIDITY: u8 = 0xF5;
/// Datasheet maximum for 14-bit temperature.
pub const TEMPERATURE_TIME: Duration = Duration::from_millis(85);
/// Datasheet maximum for 12-bit humidity.
pub const HUMIDITY_TIME: Duration = Duration::from_millis(29);
pub const RESET_TIME: Duration = Duration::from_millis(15);

pub fn soft_reset(bus: &mut dyn I2cBus) -> Result<(), BusError> {
    bus.write(ADDR, &[CMD_SOFT_RESET])
}

pub fn trigger_temperature(bus: &mut dyn I2cBus) -> Result<(), BusError> {
    bus.write(ADDR, &[CMD_TEMPERATURE])
}

pub fn trigger_humidity(bus: &mut dyn I2cBus) -> Result<(), BusError> {
    bus.write(ADDR, &[CMD_HUMIDITY])
}

pub fn fetch_temperature(bus: &mut dyn I2cBus) -> Result<f64, SensorError> {
    let mut b = [0u8; 3];
    bus.read(ADDR, &mut b)?;
    Ok(temperature(decode(&b, false)?))
}

pub fn fetch_humidity(bus: &mut dyn I2cBus) -> Result<f64, SensorError> {
    let mut b = [0u8; 3];
    bus.read(ADDR, &mut b)?;
    Ok(humidity(decode(&b, true)?))
}

/// Validate CRC and measurement type, return the raw value with status bits cleared.
pub fn decode(b: &[u8; 3], expect_humidity: bool) -> Result<u16, SensorError> {
    if crc8(&b[0..2], 0x00) != b[2] {
        return Err(SensorError::Crc);
    }
    let is_humidity = b[1] & 0x02 != 0;
    if is_humidity != expect_humidity {
        return Err(SensorError::Invalid("measurement type mismatch".into()));
    }
    Ok(u16::from_be_bytes([b[0], b[1]]) & !0x3)
}

pub fn temperature(raw: u16) -> f64 {
    -46.85 + 175.72 * f64::from(raw) / 65536.0
}

pub fn humidity(raw: u16) -> f64 {
    -6.0 + 125.0 * f64::from(raw) / 65536.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn datasheet_example() -> Result<(), SensorError> {
        // SHT2x datasheet: 0x6350 (status bits set for humidity) -> 42.5 %RH
        let raw = decode(&[0x63, 0x52, crc8(&[0x63, 0x52], 0)], true)?;
        assert!((humidity(raw) - 42.5).abs() < 0.05);
        Ok(())
    }

    #[test]
    fn rejects_wrong_type() {
        let b = [0x63, 0x50, crc8(&[0x63, 0x50], 0)];
        assert!(matches!(decode(&b, true), Err(SensorError::Invalid(_))));
    }

    #[test]
    fn temperature_conversion() {
        assert!((temperature(0x6650) - 23.3).abs() < 0.1);
    }
}
