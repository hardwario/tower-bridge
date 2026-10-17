//! Texas Instruments TMP112 temperature sensor (Climate Module R1).

use std::time::Duration;

use super::SensorError;
use crate::bus::{BusError, I2cBus};

pub const ADDR: u8 = 0x48;
pub const REG_TEMPERATURE: u8 = 0x00;
pub const REG_CONFIG: u8 = 0x01;
/// One-shot conversion while in shutdown mode (OS=1, SD=1, 4 Hz, AL=1).
pub const CONFIG_ONE_SHOT: [u8; 2] = [0xE1, 0xA0];
/// Typical conversion time is 26 ms, maximum 35 ms.
pub const CONVERSION_TIME: Duration = Duration::from_millis(35);

pub fn trigger(bus: &mut dyn I2cBus) -> Result<(), BusError> {
    bus.write(ADDR, &[REG_CONFIG, CONFIG_ONE_SHOT[0], CONFIG_ONE_SHOT[1]])
}

/// True once the one-shot conversion finished (OS and SD bits set).
pub fn is_ready(bus: &mut dyn I2cBus) -> Result<bool, BusError> {
    let mut b = [0u8; 2];
    bus.write_read(ADDR, &[REG_CONFIG], &mut b)?;
    Ok(b[0] & 0x81 == 0x81)
}

pub fn fetch(bus: &mut dyn I2cBus) -> Result<f64, SensorError> {
    let mut b = [0u8; 2];
    bus.write_read(ADDR, &[REG_TEMPERATURE], &mut b)?;
    Ok(decode(b))
}

/// 12-bit two's complement, 0.0625 °C per LSB.
pub fn decode(b: [u8; 2]) -> f64 {
    f64::from(i16::from_be_bytes(b) >> 4) * 0.0625
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn datasheet_vectors() {
        assert_eq!(decode([0xE7, 0x00]), -25.0);
        assert_eq!(decode([0x19, 0x00]), 25.0);
        assert_eq!(decode([0x00, 0x10]), 0.0625);
        assert_eq!(decode([0xFF, 0xF0]), -0.0625);
    }
}
