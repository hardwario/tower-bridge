//! Core library: TOWER Bridge Module (FT260 USB-HID to I2C) transport, TOWER
//! Climate Module sensor drivers, acquisition loop, alert engine, history store
//! and the embedded MQTT broker.

pub mod acquisition;
pub mod alerts;
#[cfg(feature = "api")]
pub mod api;
pub mod bus;
pub mod climate;
pub mod model;
pub mod relay;
pub mod sensors;

#[cfg(feature = "mqtt")]
pub mod mqtt;
#[cfg(feature = "store")]
pub mod store;
