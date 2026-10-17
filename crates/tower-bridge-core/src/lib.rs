//! Core library: TOWER Bridge Module (FT260 USB-HID to I2C) transport, TOWER
//! Climate Module and tag sensor drivers, Relay Module and NFC Tag, acquisition
//! loop, alert engine, history store and the MQTT client.

pub mod acquisition;
pub mod alerts;
#[cfg(feature = "api")]
pub mod api;
pub mod bus;
pub mod climate;
pub mod co2;
pub mod gps;
pub mod model;
pub mod nfc;
pub mod relay;
pub mod sensors;

#[cfg(feature = "mqtt")]
pub mod mqtt;
#[cfg(feature = "store")]
pub mod store;
