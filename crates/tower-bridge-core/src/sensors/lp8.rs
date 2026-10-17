//! Senseair LP8 CO2 sensor: the Modbus-like frames over its UART (9600 baud,
//! 8N2), per "LP8 sensor module specifications and integration guideline"
//! v1.02.
//!
//! One measurement is one power cycle: power on, wait for RDY low, write the
//! calculation control, the sensor state kept from the previous measurement
//! and the host pressure to RAM 0x80, read the acknowledgement, wait for RDY
//! high, read 44 bytes from 0x80, power off. Multi-byte values are big
//! endian; the CRC (Modbus CRC-16) goes low byte first.

/// "Any sensor".
const DEVICE: u8 = 0xFE;
const WRITE: u8 = 0x41;
const READ: u8 = 0x44;
const RAM: u8 = 0x80;
/// Calculation control, sensor state (23) and host pressure (2).
const WRITE_LEN: u8 = 0x1A;
/// 0x80 to 0xAB.
const READ_LEN: u8 = 0x2C;
pub const STATE_LEN: usize = 23;
/// Address, function, length, 44 data bytes, CRC.
pub const REPLY_LEN: usize = 3 + READ_LEN as usize + 2;
/// The only reply to a write.
pub const WRITE_ACK: [u8; 4] = [0xFE, 0x41, 0x81, 0xE0];
/// 1012.4 hPa, the pressure the sensor assumes without one from the host.
pub const DEFAULT_PRESSURE: u16 = 10124;

/// What the sensor does in this measurement (RAM 0x80).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Calculation {
    /// No sensor state yet: filters and the ABC baseline start over.
    Initial = 0x10,
    /// The normal case, with the previous sensor state.
    Sequential = 0x20,
    /// Automatic baseline correction (to 400 ppm, from the lowest filtered
    /// reading since the last one); the host asks for it every 8 days.
    Abc = 0x70,
}

/// Modbus CRC-16 (init 0xFFFF, reflected polynomial 0xA001).
pub fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0xFFFFu16;
    for &b in data {
        crc ^= u16::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xA001
            } else {
                crc >> 1
            };
        }
    }
    crc
}

fn with_crc(mut frame: Vec<u8>) -> Vec<u8> {
    let crc = crc16(&frame);
    frame.extend_from_slice(&crc.to_le_bytes());
    frame
}

/// The write starting a measurement. Without a sensor state it is an initial
/// measurement (the state bytes are then don't care).
pub fn write_frame(
    calculation: Calculation,
    state: Option<&[u8; STATE_LEN]>,
    pressure: u16,
) -> Vec<u8> {
    let mut frame = vec![DEVICE, WRITE, 0x00, RAM, WRITE_LEN, calculation as u8];
    frame.extend_from_slice(state.unwrap_or(&[0; STATE_LEN]));
    frame.extend_from_slice(&pressure.to_be_bytes());
    with_crc(frame)
}

/// The read after RDY went high: 44 bytes from 0x80.
pub fn read_frame() -> Vec<u8> {
    with_crc(vec![DEVICE, READ, 0x00, RAM, READ_LEN])
}

/// Bits of error status 0 (RAM 0xA7).
pub mod status0 {
    /// Configuration, memory or VCAP measurement fault: power cycle and start
    /// with an initial measurement.
    pub const FATAL: u8 = 0x01;
    /// Algorithm parameters corrupted: power cycle, initial measurement.
    pub const ALGORITHM: u8 = 0x04;
    /// A calibration or ABC was out of range; repeat it or wait for the next.
    pub const CALIBRATION: u8 = 0x08;
    /// Hardware self-diagnostics failed.
    pub const SELF_DIAG: u8 = 0x10;
    /// Somewhere in the calculation a value was out of range; clears itself.
    pub const OUT_OF_RANGE: u8 = 0x20;
    /// Virtual EEPROM or flash fault.
    pub const MEMORY: u8 = 0x40;
}

/// Bits of error status 1 (RAM 0xA6).
pub mod status1 {
    /// The capacitor was below 2.8 V before the lamp pulse.
    pub const VCAP1_LOW: u8 = 0x01;
    /// The capacitor dropped below 2.7 V during the lamp pulse.
    pub const VCAP2_LOW: u8 = 0x02;
    /// ADC out of range: power cycle, initial measurement.
    pub const ADC: u8 = 0x04;
    /// The detector signal was invalid, possibly interference.
    pub const DETECTOR: u8 = 0x08;
}

/// A decoded measurement.
#[derive(Clone, Debug, PartialEq)]
pub struct Measurement {
    /// The state to write back with the next measurement.
    pub state: [u8; STATE_LEN],
    /// Unfiltered, without pressure compensation (0x9A).
    pub co2: i16,
    /// Unfiltered, pressure compensated (0x9C).
    pub co2_pressure: i16,
    /// Sensor temperature in °C (0x9E).
    pub temperature: f64,
    /// Capacitor before and during the lamp pulse, in volts (0xA0, 0xA2).
    pub vcap1: f64,
    pub vcap2: f64,
    pub status: [u8; 4],
    /// Noise filtered (0xA8), and filtered and pressure compensated (0xAA):
    /// the value the accuracy is specified for.
    pub filtered: i16,
    pub filtered_pressure: i16,
}

impl Measurement {
    pub fn status0(&self) -> u8 {
        self.status[0]
    }

    pub fn status1(&self) -> u8 {
        self.status[1]
    }

    /// Faults after which the sensor wants a power cycle and an initial
    /// measurement (its filters are reset anyway).
    pub fn needs_restart(&self) -> bool {
        self.status0()
            & (status0::FATAL | status0::ALGORITHM | status0::MEMORY | status0::SELF_DIAG)
            != 0
            || self.status1() & status1::ADC != 0
    }

    /// The capacitor was not charged enough.
    pub fn vcap_low(&self) -> bool {
        self.status1() & (status1::VCAP1_LOW | status1::VCAP2_LOW) != 0
    }

    /// The CO2 value to report, in ppm: filtered and pressure compensated,
    /// unless this measurement says it is not to be trusted.
    pub fn ppm(&self) -> Option<f64> {
        let bad0 = self.status0() & !status0::CALIBRATION != 0;
        let bad1 = self.status1()
            & (status1::VCAP1_LOW | status1::VCAP2_LOW | status1::ADC | status1::DETECTOR)
            != 0;
        (!bad0 && !bad1 && (0..=10_000).contains(&self.filtered_pressure))
            .then(|| f64::from(self.filtered_pressure))
    }
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum FrameError {
    #[error("short reply ({0} bytes)")]
    Short(usize),
    #[error("CRC mismatch")]
    Crc,
    #[error("unexpected reply {0:02x?}")]
    Unexpected(Vec<u8>),
}

fn check(reply: &[u8], function: u8) -> Result<(), FrameError> {
    if crc16(reply) != 0 {
        return Err(FrameError::Crc);
    }
    if reply.first() != Some(&DEVICE) || reply.get(1) != Some(&function) {
        return Err(FrameError::Unexpected(
            reply.iter().take(4).copied().collect(),
        ));
    }
    Ok(())
}

/// The acknowledgement of a write (an error reply is `FE C1 <code> CRC`).
pub fn check_ack(reply: &[u8]) -> Result<(), FrameError> {
    if reply.len() < WRITE_ACK.len() {
        return Err(FrameError::Short(reply.len()));
    }
    check(reply, WRITE)
}

pub fn parse_reply(reply: &[u8]) -> Result<Measurement, FrameError> {
    if reply.len() < REPLY_LEN {
        return Err(FrameError::Short(reply.len()));
    }
    let reply = &reply[..REPLY_LEN];
    check(reply, READ)?;
    if reply[2] != READ_LEN {
        return Err(FrameError::Unexpected(reply[..4].to_vec()));
    }
    // Data byte i is RAM 0x80 + i, at reply[3 + i].
    let at = |ram: usize| reply[3 + ram - usize::from(RAM)];
    let s16 = |ram: usize| i16::from_be_bytes([at(ram), at(ram + 1)]);
    let u16v = |ram: usize| u16::from_be_bytes([at(ram), at(ram + 1)]);
    let mut state = [0u8; STATE_LEN];
    state.copy_from_slice(&reply[4..4 + STATE_LEN]);
    Ok(Measurement {
        state,
        co2: s16(0x9A),
        co2_pressure: s16(0x9C),
        temperature: f64::from(s16(0x9E)) / 100.0,
        vcap1: f64::from(u16v(0xA0)) / 1000.0,
        vcap2: f64::from(u16v(0xA2)) / 1000.0,
        status: [at(0xA7), at(0xA6), at(0xA5), at(0xA4)],
        filtered: s16(0xA8),
        filtered_pressure: s16(0xAA),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_matches_the_datasheet_examples() {
        // "Efficient initial": FE 41 00 80 01 10 28 7E, and the write ack.
        assert_eq!(
            crc16(&[0xFE, 0x41, 0x00, 0x80, 0x01, 0x10]).to_le_bytes(),
            [0x28, 0x7E]
        );
        assert_eq!(crc16(&[0xFE, 0x41]).to_le_bytes(), [0x81, 0xE0]);
        assert_eq!(crc16(&WRITE_ACK), 0);
        assert_eq!(read_frame(), vec![0xFE, 0x44, 0x00, 0x80, 0x2C, 0x79, 0x39]);
    }

    #[test]
    fn builds_the_write() {
        let state = [7u8; STATE_LEN];
        let frame = write_frame(Calculation::Sequential, Some(&state), 9850);
        assert_eq!(frame.len(), 33);
        assert_eq!(&frame[..6], &[0xFE, 0x41, 0x00, 0x80, 0x1A, 0x20]);
        assert_eq!(&frame[6..29], &state);
        assert_eq!(&frame[29..31], &9850u16.to_be_bytes());
        assert_eq!(crc16(&frame), 0, "a frame with its CRC checks to zero");
        assert_eq!(
            write_frame(Calculation::Initial, None, DEFAULT_PRESSURE)[5],
            0x10
        );
    }

    /// A reply as the sensor would send it, with these RAM values.
    fn reply(values: &[(usize, &[u8])]) -> Vec<u8> {
        let mut ram = [0u8; 44];
        for (addr, bytes) in values {
            ram[addr - 0x80..addr - 0x80 + bytes.len()].copy_from_slice(bytes);
        }
        let mut frame = vec![0xFE, 0x44, 0x2C];
        frame.extend_from_slice(&ram);
        with_crc(frame)
    }

    #[test]
    fn decodes_a_reply() -> Result<(), FrameError> {
        let r = reply(&[
            (0x81, &[1, 2, 3]),
            (0x9A, &612i16.to_be_bytes()),
            (0x9C, &605i16.to_be_bytes()),
            (0x9E, &2350i16.to_be_bytes()),
            (0xA0, &3300u16.to_be_bytes()),
            (0xA2, &3100u16.to_be_bytes()),
            (0xA8, &598i16.to_be_bytes()),
            (0xAA, &590i16.to_be_bytes()),
        ]);
        assert_eq!(r.len(), REPLY_LEN);
        let m = parse_reply(&r)?;
        assert_eq!(&m.state[..3], &[1, 2, 3]);
        assert_eq!(
            (m.co2, m.co2_pressure, m.filtered, m.filtered_pressure),
            (612, 605, 598, 590)
        );
        assert!((m.temperature - 23.5).abs() < 1e-9);
        assert!((m.vcap1 - 3.3).abs() < 1e-9 && (m.vcap2 - 3.1).abs() < 1e-9);
        assert_eq!(m.ppm(), Some(590.0));
        Ok(())
    }

    #[test]
    fn errors_withhold_the_value() -> Result<(), FrameError> {
        let ok = |s0: u8, s1: u8| -> Result<Option<f64>, FrameError> {
            Ok(parse_reply(&reply(&[
                (0xAA, &500i16.to_be_bytes()),
                (0xA7, &[s0]),
                (0xA6, &[s1]),
            ]))?
            .ppm())
        };
        assert_eq!(
            ok(status0::CALIBRATION, 0)?,
            Some(500.0),
            "a failed calibration keeps the value"
        );
        assert_eq!(ok(status0::OUT_OF_RANGE, 0)?, None);
        assert_eq!(ok(0, status1::VCAP2_LOW)?, None);
        assert_eq!(ok(0, status1::DETECTOR)?, None);
        let m = parse_reply(&reply(&[(0xA7, &[status0::FATAL])]))?;
        assert!(m.needs_restart());
        let m = parse_reply(&reply(&[(0xA6, &[status1::VCAP1_LOW])]))?;
        assert!(m.vcap_low() && !m.needs_restart());
        Ok(())
    }

    #[test]
    fn rejects_bad_replies() {
        let mut r = reply(&[]);
        r[10] ^= 1;
        assert_eq!(parse_reply(&r), Err(FrameError::Crc));
        assert_eq!(parse_reply(&[0xFE, 0x44]), Err(FrameError::Short(2)));
        assert_eq!(check_ack(&WRITE_ACK), Ok(()));
        let error = with_crc(vec![0xFE, 0xC1, 0x02]);
        assert!(matches!(check_ack(&error), Err(FrameError::Unexpected(_))));
    }
}
