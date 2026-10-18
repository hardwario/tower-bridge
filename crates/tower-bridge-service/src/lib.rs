//! The headless TOWER Bridge service, shared by the desktop app and
//! `tower-bridge-cli`: the data folder and settings, the hub that stores
//! readings, evaluates alerts and publishes to MQTT, commands to the bridge,
//! and the local API server. Front ends see it through an `Observer`.

pub mod api;
pub mod control;
mod hub;
pub mod lock;
pub mod observer;
pub mod paths;
pub mod service;
pub mod settings;
pub mod state;

pub use observer::{Observer, Silent};
pub use paths::Paths;
pub use service::{Mode, Service, ServiceConfig, StartError};
pub use settings::Settings;
pub use state::{ApiState, MqttState, ServiceState};
