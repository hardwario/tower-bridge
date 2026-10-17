//! FTDI FT260 USB-HID to I2C bridge (TOWER Bridge Module).
//!
//! Protocol per FTDI AN_394, ported from hardwario/enmon (`src/ft260.c`,
//! `src/bridge.c`). Every outgoing report is a zero-padded 64-byte buffer with
//! the report ID in byte 0; Windows requires the full report length and macOS
//! and Linux accept it.

use std::time::{Duration, Instant};

use tracing::{trace, warn};

use super::{BusError, I2cBus};
use crate::model::DeviceInfo;

pub const VID: u16 = 0x0403;
pub const PID: u16 = 0x6030;
/// USB interface carrying the I2C function.
pub const I2C_INTERFACE: i32 = 0;

pub const REPORT_LEN: usize = 64;
/// Payload bytes that fit in one I2C data report.
pub const MAX_CHUNK: usize = 60;

const RID_CHIP_VERSION: u8 = 0xA0;
const RID_SYSTEM: u8 = 0xA1;
const RID_GPIO: u8 = 0xB0;
const RID_I2C_STATUS: u8 = 0xC0;
const RID_I2C_READ_REQUEST: u8 = 0xC2;
const RID_I2C_DATA: u8 = 0xD0;

const SYS_SET_CLOCK: u8 = 0x01;
const SYS_CLOCK_48MHZ: u8 = 0x02;
const SYS_SET_I2C_MODE: u8 = 0x02;
const SYS_SET_UART_MODE: u8 = 0x03;
const SYS_UART_OFF: u8 = 0x00;
const SYS_I2C_RESET: u8 = 0x20;
const SYS_SET_I2C_SPEED: u8 = 0x22;

/// I2C condition flags.
pub const FLAG_START: u8 = 0x02;
pub const FLAG_START_STOP: u8 = 0x06;
pub const FLAG_REPEATED_START_STOP: u8 = 0x07;

/// I2C status bits (report 0xC0, byte 1).
pub const ST_BUSY: u8 = 0x01;
pub const ST_ERROR: u8 = 0x02;
pub const ST_ADDR_NACK: u8 = 0x04;
pub const ST_DATA_NACK: u8 = 0x08;
pub const ST_ARB_LOST: u8 = 0x10;
pub const ST_IDLE: u8 = 0x20;
pub const ST_BUS_BUSY: u8 = 0x40;
const ST_ERROR_MASK: u8 = ST_ERROR | ST_ADDR_NACK | ST_DATA_NACK | ST_ARB_LOST;

/// I2C clock in kHz.
pub const I2C_SPEED_KHZ: u16 = 100;

/// The bridge LED sits on GPIOH (bit 7 of the extended GPIO bytes), active high.
const GPIO_LED_MASK: u8 = 0x80;

/// Delays and timeouts. The defaults are enmon's proven values; enmon has no
/// status timeout, which is added here.
#[derive(Clone, Copy, Debug)]
pub struct Timing {
    pub pre_transfer: Duration,
    pub status_poll: Duration,
    pub status_timeout: Duration,
    pub input_report_timeout_ms: i32,
}

impl Timing {
    pub const DEFAULT: Timing = Timing {
        pre_transfer: Duration::from_millis(10),
        status_poll: Duration::from_millis(10),
        status_timeout: Duration::from_millis(200),
        input_report_timeout_ms: 500,
    };
}

impl Default for Timing {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Raw HID operations the FT260 driver needs. Implemented for
/// [`hidapi::HidDevice`]; tests substitute a fake.
pub trait HidTransport: Send {
    /// Send an output report (`data[0]` is the report ID).
    fn write(&mut self, data: &[u8]) -> Result<(), BusError>;
    /// Read an input report; returns the number of bytes, 0 on timeout.
    fn read_timeout(&mut self, buf: &mut [u8], timeout_ms: i32) -> Result<usize, BusError>;
    /// Send a feature report (`data[0]` is the report ID).
    fn send_feature(&mut self, data: &[u8]) -> Result<(), BusError>;
    /// Get a feature report; `buf[0]` holds the report ID on entry.
    fn get_feature(&mut self, buf: &mut [u8]) -> Result<usize, BusError>;
}

impl HidTransport for hidapi::HidDevice {
    fn write(&mut self, data: &[u8]) -> Result<(), BusError> {
        let n = hidapi::HidDevice::write(self, data).map_err(hid_err)?;
        if n < data.len() {
            return Err(BusError::Hid(format!(
                "short write ({n} of {})",
                data.len()
            )));
        }
        Ok(())
    }

    fn read_timeout(&mut self, buf: &mut [u8], timeout_ms: i32) -> Result<usize, BusError> {
        hidapi::HidDevice::read_timeout(self, buf, timeout_ms).map_err(hid_err)
    }

    fn send_feature(&mut self, data: &[u8]) -> Result<(), BusError> {
        self.send_feature_report(data).map_err(hid_err)
    }

    fn get_feature(&mut self, buf: &mut [u8]) -> Result<usize, BusError> {
        self.get_feature_report(buf).map_err(hid_err)
    }
}

fn hid_err(e: hidapi::HidError) -> BusError {
    BusError::Hid(e.to_string())
}

/// Report ID of an I2C data report carrying `len` bytes.
pub fn data_report_id(len: usize) -> u8 {
    RID_I2C_DATA + ((len.max(1) - 1) / 4) as u8
}

/// Build an I2C write report.
pub fn build_write_report(addr: u8, data: &[u8], flag: u8) -> Result<[u8; REPORT_LEN], BusError> {
    if data.is_empty() || data.len() > MAX_CHUNK {
        return Err(BusError::Protocol(format!(
            "write length {} out of range",
            data.len()
        )));
    }
    let mut r = [0u8; REPORT_LEN];
    r[0] = data_report_id(data.len());
    r[1] = addr;
    r[2] = flag;
    r[3] = data.len() as u8;
    r[4..4 + data.len()].copy_from_slice(data);
    Ok(r)
}

/// Build an I2C read request report.
pub fn build_read_request(addr: u8, len: usize, flag: u8) -> [u8; REPORT_LEN] {
    let mut r = [0u8; REPORT_LEN];
    r[0] = RID_I2C_READ_REQUEST;
    r[1] = addr;
    r[2] = flag;
    r[3] = (len & 0xff) as u8;
    r[4] = (len >> 8) as u8;
    r
}

/// Outcome of one I2C status poll.
#[derive(Debug, PartialEq, Eq)]
pub enum StatusOutcome {
    /// Keep polling.
    Pending,
    /// Transfer finished successfully.
    Done,
    Failed(BusError),
}

/// Interpret an I2C status byte. `bus_held` is true after a transfer that
/// ended without STOP (the bus must remain busy).
pub fn decode_status(status: u8, bus_held: bool) -> StatusOutcome {
    if status & ST_BUSY != 0 {
        return StatusOutcome::Pending;
    }
    if status & ST_ERROR_MASK != 0 {
        let e = if status & ST_ADDR_NACK != 0 {
            BusError::AddrNack
        } else if status & ST_DATA_NACK != 0 {
            BusError::DataNack
        } else if status & ST_ARB_LOST != 0 {
            BusError::ArbitrationLost
        } else {
            BusError::Bus(status)
        };
        return StatusOutcome::Failed(e);
    }
    if bus_held {
        if status & ST_BUS_BUSY == 0 {
            return StatusOutcome::Failed(BusError::Protocol(format!(
                "bus released unexpectedly (status 0x{status:02x})"
            )));
        }
        return StatusOutcome::Done;
    }
    if status & ST_BUS_BUSY != 0 {
        return StatusOutcome::Pending;
    }
    if status & ST_IDLE == 0 {
        return StatusOutcome::Failed(BusError::Protocol(format!(
            "controller not idle (status 0x{status:02x})"
        )));
    }
    StatusOutcome::Done
}

fn hex(data: &[u8]) -> String {
    data.iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Hex dump without the zero padding at the end (feature reports).
fn hex_trimmed(data: &[u8]) -> String {
    let end = data.iter().rposition(|b| *b != 0).map_or(1, |i| i + 1);
    hex(&data[..end.min(data.len())])
}

/// FT260 I2C master.
pub struct Ft260<T: HidTransport> {
    dev: T,
    info: DeviceInfo,
    timing: Timing,
}

impl<T: HidTransport> Ft260<T> {
    pub fn new(dev: T, info: DeviceInfo) -> Self {
        Self {
            dev,
            info,
            timing: Timing::DEFAULT,
        }
    }

    pub fn with_timing(mut self, timing: Timing) -> Self {
        self.timing = timing;
        self
    }

    /// Run enmon's initialisation sequence: read chip version and system
    /// status, 48 MHz clock, enable I2C, UART off, I2C reset, set the I2C clock.
    pub fn initialize(&mut self) -> Result<(), BusError> {
        let version = self.feature_get(RID_CHIP_VERSION)?;
        self.info.chip = Some(format!(
            "{:02x}{:02x} rev {:02x}{:02x}",
            version[1], version[2], version[3], version[4]
        ));
        self.feature_get(RID_SYSTEM)?;
        self.feature_set(&[RID_SYSTEM, SYS_SET_CLOCK, SYS_CLOCK_48MHZ])?;
        self.feature_set(&[RID_SYSTEM, SYS_SET_I2C_MODE, 0x01])?;
        self.feature_set(&[RID_SYSTEM, SYS_SET_UART_MODE, SYS_UART_OFF])?;
        self.i2c_reset()?;
        let [lo, hi] = I2C_SPEED_KHZ.to_le_bytes();
        self.feature_set(&[RID_SYSTEM, SYS_SET_I2C_SPEED, lo, hi])?;
        Ok(())
    }

    /// Cheap liveness check (reads the chip version report).
    pub fn ping(&mut self) -> Result<(), BusError> {
        self.feature_get(RID_CHIP_VERSION).map(|_| ())
    }

    fn feature_set(&mut self, payload: &[u8]) -> Result<(), BusError> {
        let mut r = [0u8; REPORT_LEN];
        r[..payload.len()].copy_from_slice(payload);
        trace!(target: "ft260", "feature-set {}", hex(payload));
        self.dev.send_feature(&r)
    }

    fn feature_get(&mut self, id: u8) -> Result<[u8; REPORT_LEN], BusError> {
        let mut r = [0u8; REPORT_LEN];
        r[0] = id;
        let n = self.dev.get_feature(&mut r)?;
        trace!(target: "ft260", "feature-get {id:02x} -> {}", hex_trimmed(&r[..n.min(REPORT_LEN)]));
        if n < 2 || r[0] != id {
            return Err(BusError::Protocol(format!(
                "bad feature report 0x{id:02x} (len {n}, id 0x{:02x})",
                r[0]
            )));
        }
        Ok(r)
    }

    fn i2c_reset(&mut self) -> Result<(), BusError> {
        self.feature_set(&[RID_SYSTEM, SYS_I2C_RESET])
    }

    fn i2c_status(&mut self) -> Result<u8, BusError> {
        Ok(self.feature_get(RID_I2C_STATUS)?[1])
    }

    fn wait(&mut self, bus_held: bool) -> Result<(), BusError> {
        let start = Instant::now();
        loop {
            std::thread::sleep(self.timing.status_poll);
            let status = self.i2c_status()?;
            match decode_status(status, bus_held) {
                StatusOutcome::Done => return Ok(()),
                StatusOutcome::Failed(e) => {
                    if status & ST_BUS_BUSY != 0 {
                        // Free a bus left hanging by the failed transfer.
                        self.i2c_reset()?;
                    }
                    return Err(e);
                }
                StatusOutcome::Pending => {
                    if start.elapsed() > self.timing.status_timeout {
                        self.i2c_reset()?;
                        return Err(BusError::Timeout);
                    }
                }
            }
        }
    }

    fn raw_write(&mut self, addr: u8, data: &[u8], flag: u8) -> Result<(), BusError> {
        std::thread::sleep(self.timing.pre_transfer);
        let r = build_write_report(addr, data, flag)?;
        trace!(target: "ft260", "out {}", hex(&r[..4 + data.len()]));
        self.dev.write(&r)?;
        self.wait(flag & 0x04 == 0)
    }

    fn raw_read(&mut self, addr: u8, buf: &mut [u8], flag: u8) -> Result<(), BusError> {
        if buf.is_empty() || buf.len() > MAX_CHUNK {
            return Err(BusError::Protocol(format!(
                "read length {} out of range",
                buf.len()
            )));
        }
        if flag != FLAG_REPEATED_START_STOP {
            std::thread::sleep(self.timing.pre_transfer);
        }
        // Discard stale input reports left over from an earlier failure.
        let mut scratch = [0u8; REPORT_LEN];
        for _ in 0..8 {
            if self.dev.read_timeout(&mut scratch, 0)? == 0 {
                break;
            }
        }
        let req = build_read_request(addr, buf.len(), flag);
        trace!(target: "ft260", "out {}", hex(&req[..5]));
        self.dev.write(&req)?;

        let mut r = [0u8; REPORT_LEN];
        let n = self
            .dev
            .read_timeout(&mut r, self.timing.input_report_timeout_ms)?;
        trace!(target: "ft260", "in  {}", hex(&r[..n.min(2 + buf.len())]));
        if n == 0 {
            // Typically the address was not acknowledged; the status tells.
            self.wait(false)?;
            self.i2c_reset()?;
            return Err(BusError::Timeout);
        }
        self.wait(false)?;
        if n < buf.len() + 2 {
            return Err(BusError::Protocol(format!(
                "short input report ({n} bytes)"
            )));
        }
        if r[0] != data_report_id(buf.len()) || r[1] as usize != buf.len() {
            return Err(BusError::Protocol(format!(
                "unexpected input report id 0x{:02x} len {}",
                r[0], r[1]
            )));
        }
        buf.copy_from_slice(&r[2..2 + buf.len()]);
        Ok(())
    }

    fn with_retry<R>(
        &mut self,
        mut op: impl FnMut(&mut Self) -> Result<R, BusError>,
    ) -> Result<R, BusError> {
        match op(self) {
            Err(e) if e.is_transient() => {
                warn!(target: "ft260", "I2C transfer failed ({e}), resetting controller and retrying");
                self.i2c_reset()?;
                op(self)
            }
            r => r,
        }
    }
}

impl<T: HidTransport> I2cBus for Ft260<T> {
    fn write(&mut self, addr: u8, data: &[u8]) -> Result<(), BusError> {
        self.with_retry(|s| s.raw_write(addr, data, FLAG_START_STOP))
    }

    fn read(&mut self, addr: u8, buf: &mut [u8]) -> Result<(), BusError> {
        self.with_retry(|s| s.raw_read(addr, buf, FLAG_START_STOP))
    }

    fn write_read(&mut self, addr: u8, wr: &[u8], rd: &mut [u8]) -> Result<(), BusError> {
        self.with_retry(|s| {
            s.raw_write(addr, wr, FLAG_START)?;
            s.raw_read(addr, rd, FLAG_REPEATED_START_STOP)
        })
    }

    fn recover(&mut self) -> Result<(), BusError> {
        self.i2c_reset()
    }

    fn set_led(&mut self, on: bool) -> Result<(), BusError> {
        // [id, GPIO0-5 value, GPIO0-5 direction, GPIOA-H value, GPIOA-H direction]
        self.feature_set(&[
            RID_GPIO,
            0x00,
            0x00,
            if on { GPIO_LED_MASK } else { 0x00 },
            GPIO_LED_MASK,
        ])
    }

    fn info(&self) -> &DeviceInfo {
        &self.info
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;
    use crate::model::Backend;

    #[derive(Default)]
    struct FakeHid {
        writes: Vec<Vec<u8>>,
        features_sent: Vec<Vec<u8>>,
        statuses: VecDeque<u8>,
        /// Status returned once `statuses` is exhausted (idle by default).
        stuck_status: Option<u8>,
        inputs: VecDeque<Vec<u8>>,
    }

    impl HidTransport for FakeHid {
        fn write(&mut self, data: &[u8]) -> Result<(), BusError> {
            self.writes.push(data.to_vec());
            Ok(())
        }

        fn read_timeout(&mut self, buf: &mut [u8], timeout_ms: i32) -> Result<usize, BusError> {
            if timeout_ms == 0 {
                return Ok(0);
            }
            match self.inputs.pop_front() {
                Some(r) => {
                    buf[..r.len()].copy_from_slice(&r);
                    Ok(r.len())
                }
                None => Ok(0),
            }
        }

        fn send_feature(&mut self, data: &[u8]) -> Result<(), BusError> {
            self.features_sent.push(data.to_vec());
            Ok(())
        }

        fn get_feature(&mut self, buf: &mut [u8]) -> Result<usize, BusError> {
            match buf[0] {
                RID_I2C_STATUS => {
                    buf[1] = self
                        .statuses
                        .pop_front()
                        .or(self.stuck_status)
                        .unwrap_or(ST_IDLE);
                    Ok(5)
                }
                RID_CHIP_VERSION => {
                    buf[1..5].copy_from_slice(&[0x02, 0x60, 0x01, 0x00]);
                    Ok(13)
                }
                _ => Ok(25),
            }
        }
    }

    fn info() -> DeviceInfo {
        DeviceInfo {
            backend: Backend::Hid,
            serial: "TEST".into(),
            path: "fake".into(),
            chip: None,
        }
    }

    fn zero_timing() -> Timing {
        Timing {
            pre_transfer: Duration::ZERO,
            status_poll: Duration::ZERO,
            status_timeout: Duration::from_millis(50),
            input_report_timeout_ms: 1,
        }
    }

    #[test]
    fn write_report_layout() -> Result<(), BusError> {
        let r = build_write_report(0x45, &[0x24, 0x00], FLAG_START_STOP)?;
        assert_eq!(&r[..6], &[0xD0, 0x45, 0x06, 0x02, 0x24, 0x00]);
        assert!(r[6..].iter().all(|b| *b == 0));
        let r = build_write_report(0x44, &[1, 2, 3, 4, 5], FLAG_START)?;
        assert_eq!(r[0], 0xD1);
        assert!(build_write_report(0x44, &[], FLAG_START).is_err());
        Ok(())
    }

    #[test]
    fn read_request_layout() {
        let r = build_read_request(0x60, 5, FLAG_REPEATED_START_STOP);
        assert_eq!(&r[..5], &[0xC2, 0x60, 0x07, 0x05, 0x00]);
    }

    #[test]
    fn status_decoding() {
        assert_eq!(decode_status(ST_BUSY, false), StatusOutcome::Pending);
        assert_eq!(decode_status(ST_IDLE, false), StatusOutcome::Done);
        assert_eq!(
            decode_status(ST_IDLE | ST_BUS_BUSY, false),
            StatusOutcome::Pending
        );
        assert_eq!(decode_status(ST_BUS_BUSY, true), StatusOutcome::Done);
        assert_eq!(
            decode_status(ST_ERROR | ST_ADDR_NACK | ST_IDLE, false),
            StatusOutcome::Failed(BusError::AddrNack)
        );
        assert_eq!(
            decode_status(ST_ERROR | ST_DATA_NACK, false),
            StatusOutcome::Failed(BusError::DataNack)
        );
        assert_eq!(
            decode_status(ST_ERROR | ST_ARB_LOST, false),
            StatusOutcome::Failed(BusError::ArbitrationLost)
        );
        assert!(matches!(
            decode_status(ST_IDLE, true),
            StatusOutcome::Failed(BusError::Protocol(_))
        ));
    }

    #[test]
    fn initialize_sends_enmon_sequence() -> Result<(), BusError> {
        let mut ft = Ft260::new(FakeHid::default(), info()).with_timing(zero_timing());
        ft.initialize()?;
        let sent: Vec<Vec<u8>> = ft
            .dev
            .features_sent
            .iter()
            .map(|r| r[..5].to_vec())
            .collect();
        assert_eq!(
            sent,
            vec![
                vec![0xA1, 0x01, 0x02, 0, 0],
                vec![0xA1, 0x02, 0x01, 0, 0],
                vec![0xA1, 0x03, 0x00, 0, 0],
                vec![0xA1, 0x20, 0, 0, 0],
                vec![0xA1, 0x22, 0x64, 0x00, 0],
            ]
        );
        assert!(ft.dev.features_sent.iter().all(|r| r.len() == REPORT_LEN));
        assert_eq!(ft.info().chip.as_deref(), Some("0260 rev 0100"));
        Ok(())
    }

    #[test]
    fn write_read_uses_repeated_start() -> Result<(), BusError> {
        let mut hid = FakeHid::default();
        hid.statuses.extend([ST_BUS_BUSY, ST_IDLE]);
        hid.inputs.push_back(vec![0xD0, 2, 0xAB, 0xCD]);
        let mut ft = Ft260::new(hid, info()).with_timing(zero_timing());
        let mut buf = [0u8; 2];
        ft.write_read(0x44, &[0x00], &mut buf)?;
        assert_eq!(buf, [0xAB, 0xCD]);
        assert_eq!(&ft.dev.writes[0][..5], &[0xD0, 0x44, 0x02, 0x01, 0x00]);
        assert_eq!(&ft.dev.writes[1][..5], &[0xC2, 0x44, 0x07, 0x02, 0x00]);
        Ok(())
    }

    #[test]
    fn nack_is_reported_without_retry() {
        let mut hid = FakeHid::default();
        hid.statuses.push_back(ST_ERROR | ST_ADDR_NACK | ST_IDLE);
        let mut ft = Ft260::new(hid, info()).with_timing(zero_timing());
        assert_eq!(ft.write(0x45, &[0x30, 0xA2]), Err(BusError::AddrNack));
        assert_eq!(ft.dev.writes.len(), 1);
    }

    #[test]
    fn stuck_controller_times_out_and_retries_once() {
        let hid = FakeHid {
            stuck_status: Some(ST_BUSY),
            ..FakeHid::default()
        };
        let mut ft = Ft260::new(hid, info()).with_timing(zero_timing());
        assert_eq!(ft.write(0x45, &[0x30, 0xA2]), Err(BusError::Timeout));
        assert_eq!(ft.dev.writes.len(), 2);
    }
}
