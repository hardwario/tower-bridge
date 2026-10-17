//! I2C bus abstraction and its backends.

pub mod discovery;
pub mod ft260;
#[cfg(target_os = "linux")]
pub mod linux_i2cdev;
pub mod mock;

use std::time::{Duration, Instant};

use crate::model::DeviceInfo;

/// I2C transfer failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BusError {
    /// USB/HID level failure; the device must be reopened.
    #[error("USB HID error: {0}")]
    Hid(String),
    /// OS level I/O failure (i2c-dev); the device must be reopened.
    #[error("I/O error: {0}")]
    Io(String),
    /// No device acknowledged the address.
    #[error("address not acknowledged")]
    AddrNack,
    /// The device stopped acknowledging data bytes.
    #[error("data not acknowledged")]
    DataNack,
    #[error("arbitration lost")]
    ArbitrationLost,
    /// The bridge reported a bus error.
    #[error("bus error (status 0x{0:02x})")]
    Bus(u8),
    /// The transfer did not complete in time.
    #[error("timeout")]
    Timeout,
    /// Unexpected response from the bridge.
    #[error("protocol error: {0}")]
    Protocol(String),
}

impl BusError {
    /// The transport itself failed and must be reopened.
    pub fn is_fatal(&self) -> bool {
        matches!(self, BusError::Hid(_) | BusError::Io(_))
    }

    /// Worth one recovery attempt and retry.
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            BusError::ArbitrationLost
                | BusError::Bus(_)
                | BusError::Timeout
                | BusError::Protocol(_)
        )
    }
}

/// Minimal I2C master interface used by the sensor drivers.
///
/// Addresses are 7-bit (not shifted).
pub trait I2cBus: Send {
    /// Write `data` (START, address+W, data, STOP).
    fn write(&mut self, addr: u8, data: &[u8]) -> Result<(), BusError>;
    /// Read into `buf` (START, address+R, data, STOP).
    fn read(&mut self, addr: u8, buf: &mut [u8]) -> Result<(), BusError>;
    /// Write `wr` then read into `rd` using a repeated START in between.
    fn write_read(&mut self, addr: u8, wr: &[u8], rd: &mut [u8]) -> Result<(), BusError>;
    /// Reset the I2C controller after an error.
    fn recover(&mut self) -> Result<(), BusError>;
    /// Drive the bridge LED, when the backend supports it.
    fn set_led(&mut self, _on: bool) -> Result<(), BusError> {
        Ok(())
    }
    /// Identity of the opened device.
    fn info(&self) -> &DeviceInfo;
}

/// Time source used by the measurement logic, so tests can run without sleeping.
pub trait Clock: Send {
    /// Monotonic time since an arbitrary origin.
    fn now(&self) -> Duration;
    fn sleep(&self, d: Duration);

    /// Sleep until `now() >= deadline`.
    fn sleep_until(&self, deadline: Duration) {
        let now = self.now();
        if deadline > now {
            self.sleep(deadline - now);
        }
    }
}

/// Real wall-clock time.
pub struct SystemClock {
    origin: Instant,
}

impl SystemClock {
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }

    fn sleep(&self, d: Duration) {
        std::thread::sleep(d);
    }
}
