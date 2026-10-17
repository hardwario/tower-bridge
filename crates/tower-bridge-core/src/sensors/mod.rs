//! Drivers for the sensors on the TOWER Climate Module.
//!
//! Each driver exposes the individual bus steps (trigger, ready check, fetch)
//! so that [`crate::climate`] can run all conversions concurrently, plus pure
//! decode functions that are unit tested against datasheet vectors.

pub mod crc;
pub mod identify;
pub mod lp8;
pub mod mpl3115a2;
pub mod opt3001;
pub mod sc16is740;
pub mod sgpc3;
pub mod sht20;
pub mod sht30;
pub mod tmp112;

use crate::bus::BusError;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SensorError {
    #[error(transparent)]
    Bus(#[from] BusError),
    #[error("CRC mismatch")]
    Crc,
    #[error("measurement not ready")]
    NotReady,
    #[error("unexpected data: {0}")]
    Invalid(String),
}

impl SensorError {
    /// The transport failed; the device must be reopened.
    pub fn is_fatal(&self) -> bool {
        matches!(self, SensorError::Bus(e) if e.is_fatal())
    }
}
