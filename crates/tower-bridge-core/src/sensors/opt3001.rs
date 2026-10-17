//! Texas Instruments OPT3001 ambient light sensor.

use std::time::Duration;

use super::SensorError;
use crate::bus::{BusError, I2cBus};

/// Default address (ADDR to GND), and the Lux Meter Tag's alternative.
pub const ADDR: u8 = 0x44;
pub const ALT_ADDR: u8 = 0x45;
pub const REG_RESULT: u8 = 0x00;
pub const REG_CONFIG: u8 = 0x01;
pub const REG_MANUFACTURER_ID: u8 = 0x7E;
pub const REG_DEVICE_ID: u8 = 0x7F;
pub const DEVICE_ID: u16 = 0x3001;
/// Automatic full-scale, 800 ms conversion, shutdown, latched.
pub const CONFIG_SHUTDOWN: [u8; 2] = [0xC8, 0x10];
/// Same settings, single-shot conversion.
pub const CONFIG_SINGLE_SHOT: [u8; 2] = [0xCA, 0x10];
pub const CONVERSION_TIME: Duration = Duration::from_millis(880);

/// Device ID register; [`DEVICE_ID`] for an OPT3001 (an SHT3x can share the address).
pub fn device_id(bus: &mut dyn I2cBus, addr: u8) -> Result<u16, BusError> {
    let mut b = [0u8; 2];
    bus.write_read(addr, &[REG_DEVICE_ID], &mut b)?;
    Ok(u16::from_be_bytes(b))
}

pub fn init(bus: &mut dyn I2cBus, addr: u8) -> Result<(), BusError> {
    bus.write(addr, &[REG_CONFIG, CONFIG_SHUTDOWN[0], CONFIG_SHUTDOWN[1]])
}

pub fn trigger(bus: &mut dyn I2cBus, addr: u8) -> Result<(), BusError> {
    bus.write(
        addr,
        &[REG_CONFIG, CONFIG_SINGLE_SHOT[0], CONFIG_SINGLE_SHOT[1]],
    )
}

/// True once the device returned to shutdown with the conversion-ready flag set.
pub fn is_ready(bus: &mut dyn I2cBus, addr: u8) -> Result<bool, BusError> {
    let mut b = [0u8; 2];
    bus.write_read(addr, &[REG_CONFIG], &mut b)?;
    Ok(b[0] & 0x06 == 0 && b[1] & 0x80 != 0)
}

pub fn fetch(bus: &mut dyn I2cBus, addr: u8) -> Result<f64, SensorError> {
    let mut b = [0u8; 2];
    bus.write_read(addr, &[REG_RESULT], &mut b)?;
    Ok(lux(u16::from_be_bytes(b)))
}

/// lux = 0.01 * 2^E * R
pub fn lux(raw: u16) -> f64 {
    let exponent = raw >> 12;
    let mantissa = raw & 0x0FFF;
    0.01 * f64::from(1u32 << exponent) * f64::from(mantissa)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn datasheet_vectors() {
        assert!((lux(0x6542) - 861.44).abs() < 1e-9);
        assert!((lux(0x0FFF) - 40.95).abs() < 1e-9);
        assert!((lux(0xBFFF) - 83865.6).abs() < 1e-6);
    }
}
