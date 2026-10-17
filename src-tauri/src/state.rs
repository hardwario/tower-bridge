//! Shared application state.

use std::sync::Arc;

use crossbeam_channel::Sender;
use parking_lot::RwLock;
use serde::Serialize;
use tower_bridge_core::acquisition::AcqCommand;
use tower_bridge_core::alerts::ActiveAlert;
use tower_bridge_core::model::{DeviceState, Reading};
use tower_bridge_core::relay::RelayInfo;

use crate::hub::HubMsg;
use crate::paths::Paths;
use crate::settings::Settings;
use crate::tray::TrayController;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum MqttState {
    Disabled,
    #[serde(rename_all = "snake_case")]
    Running {
        address: String,
    },
    #[serde(rename_all = "snake_case")]
    Error {
        message: String,
    },
    /// Settings changed in a way that needs an app restart (the broker
    /// cannot be stopped or re-bound while running).
    #[serde(rename_all = "snake_case")]
    RestartRequired {
        reason: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ApiState {
    Disabled,
    #[serde(rename_all = "snake_case")]
    Running {
        /// Listening address, e.g. `127.0.0.1:8180` or `0.0.0.0:8180`.
        address: String,
        /// URL usable from this computer.
        url: String,
    },
    #[serde(rename_all = "snake_case")]
    Error {
        port: u16,
        message: String,
    },
}

pub struct AppState {
    pub settings: RwLock<Arc<Settings>>,
    pub latest: RwLock<Option<Reading>>,
    pub device: RwLock<DeviceState>,
    /// Detected Relay Module, if any.
    pub relay: RwLock<Option<RelayInfo>>,
    pub alerts: RwLock<Vec<ActiveAlert>>,
    pub mqtt: RwLock<MqttState>,
    pub api: RwLock<ApiState>,
    pub paths: Paths,
    pub simulated: bool,
    pub acq_tx: Sender<AcqCommand>,
    pub hub_tx: Sender<HubMsg>,
    pub tray: TrayController,
}

impl AppState {
    pub fn settings(&self) -> Arc<Settings> {
        self.settings.read().clone()
    }
}
