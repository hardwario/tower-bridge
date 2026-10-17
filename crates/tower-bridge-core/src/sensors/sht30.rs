//! Sensirion SHT30 temperature and humidity sensor (Climate Module R2).

use std::time::Duration;

use super::SensorError;
use super::crc::crc8;
use crate::bus::{BusError, I2cBus};

pub const ADDR: u8 = 0x45;
pub const CMD_SOFT_RESET: [u8; 2] = [0x30, 0xA2];
/// Single shot, high repeatability, no clock stretching.
pub const CMD_MEASURE: [u8; 2] = [0x24, 0x00];
/// Datasheet maximum is 15.5 ms.
pub const MEASURE_TIME: Duration = Duration::from_millis(20);
pub const RESET_TIME: Duration = Duration::from_millis(2);

pub fn soft_reset(bus: &mut dyn I2cBus) -> Result<(), BusError> {
    bus.write(ADDR, &CMD_SOFT_RESET)
}

pub fn trigger(bus: &mut dyn I2cBus) -> Result<(), BusError> {
    bus.write(ADDR, &CMD_MEASURE)
}

/// Read the result of a triggered measurement: (°C, %RH).
pub fn fetch(bus: &mut dyn I2cBus) -> Result<(f64, f64), SensorError> {
    let mut b = [0u8; 6];
    bus.read(ADDR, &mut b)?;
    decode(&b)
}

/// Decode `[T msb, T lsb, T crc, RH msb, RH lsb, RH crc]`.
pub fn decode(b: &[u8; 6]) -> Result<(f64, f64), SensorError> {
    if crc8(&b[0..2], 0xFF) != b[2] || crc8(&b[3..5], 0xFF) != b[5] {
        return Err(SensorError::Crc);
    }
    let t = u16::from_be_bytes([b[0], b[1]]);
    let rh = u16::from_be_bytes([b[3], b[4]]);
    Ok((temperature(t), humidity(rh)))
}

pub fn temperature(raw: u16) -> f64 {
    -45.0 + 175.0 * f64::from(raw) / 65535.0
}

pub fn humidity(raw: u16) -> f64 {
    100.0 * f64::from(raw) / 65535.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversion() {
        assert!((temperature(0x6666) - 25.0).abs() < 0.01);
        assert!((humidity(0x8000) - 50.0).abs() < 0.01);
    }

    #[test]
    fn decode_checks_crc() {
        let t = 0x6666u16.to_be_bytes();
        let rh = 0x8000u16.to_be_bytes();
        let mut b = [t[0], t[1], crc8(&t, 0xFF), rh[0], rh[1], crc8(&rh, 0xFF)];
        assert!(decode(&b).is_ok());
        b[4] ^= 1;
        assert_eq!(decode(&b), Err(SensorError::Crc));
    }
}
