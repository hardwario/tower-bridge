//! NXP MPL3115A2 barometric pressure sensor.

use std::time::Duration;

use super::SensorError;
use crate::bus::{BusError, I2cBus};

pub const ADDR: u8 = 0x60;
pub const REG_STATUS: u8 = 0x00;
pub const REG_OUT_P_MSB: u8 = 0x01;
pub const REG_WHO_AM_I: u8 = 0x0C;
pub const REG_PT_DATA_CFG: u8 = 0x13;
pub const REG_CTRL_REG1: u8 = 0x26;
pub const WHO_AM_I: u8 = 0xC4;
pub const CTRL_RESET: u8 = 0x04;
/// Barometer mode, oversampling 128, standby.
pub const CTRL_BAROMETER: u8 = 0x38;
pub const CTRL_ONE_SHOT: u8 = 0x02;
/// Data-ready event flags for pressure and temperature.
pub const PT_DATA_CFG_ALL: u8 = 0x07;
pub const STATUS_PDR: u8 = 0x04;
pub const RESET_TIME: Duration = Duration::from_millis(1000);
/// Conversion time at oversampling 128.
pub const CONVERSION_TIME: Duration = Duration::from_millis(550);

/// Software reset. The device does not acknowledge the write while it
/// resets, so errors are ignored.
pub fn reset(bus: &mut dyn I2cBus) {
    let _ = bus.write(ADDR, &[REG_CTRL_REG1, CTRL_RESET]);
}

pub fn who_am_i(bus: &mut dyn I2cBus) -> Result<u8, BusError> {
    let mut b = [0u8; 1];
    bus.write_read(ADDR, &[REG_WHO_AM_I], &mut b)?;
    Ok(b[0])
}

pub fn configure(bus: &mut dyn I2cBus) -> Result<(), BusError> {
    bus.write(ADDR, &[REG_CTRL_REG1, CTRL_BAROMETER])?;
    bus.write(ADDR, &[REG_PT_DATA_CFG, PT_DATA_CFG_ALL])
}

pub fn trigger(bus: &mut dyn I2cBus) -> Result<(), BusError> {
    bus.write(ADDR, &[REG_CTRL_REG1, CTRL_BAROMETER | CTRL_ONE_SHOT])
}

pub fn is_ready(bus: &mut dyn I2cBus) -> Result<bool, BusError> {
    let mut b = [0u8; 1];
    bus.write_read(ADDR, &[REG_STATUS], &mut b)?;
    Ok(b[0] & STATUS_PDR != 0)
}

/// Read pressure (Pa) and die temperature (°C).
pub fn fetch(bus: &mut dyn I2cBus) -> Result<(f64, f64), SensorError> {
    let mut b = [0u8; 5];
    bus.write_read(ADDR, &[REG_OUT_P_MSB], &mut b)?;
    Ok((pressure([b[0], b[1], b[2]]), temperature([b[3], b[4]])))
}

/// Unsigned Q18.2 pascals in the upper 20 bits.
pub fn pressure(b: [u8; 3]) -> f64 {
    let raw = (u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2])) >> 4;
    f64::from(raw) / 4.0
}

/// Signed Q8.4 degrees Celsius.
pub fn temperature(b: [u8; 2]) -> f64 {
    f64::from(i16::from_be_bytes(b) >> 4) / 16.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pressure_vector() {
        assert_eq!(pressure([0x63, 0x05, 0x80]), 101_398.0);
        assert_eq!(pressure([0x63, 0x05, 0x90]), 101_398.25);
    }

    #[test]
    fn temperature_vector() {
        assert_eq!(temperature([0x17, 0x80]), 23.5);
        assert_eq!(temperature([0xFF, 0xF0]), -0.0625);
    }
}
