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
    /// Connecting to the broker, or again after it dropped; `error` says why
    /// the last attempt failed.
    #[serde(rename_all = "snake_case")]
    Connecting {
        address: String,
        error: Option<String>,
    },
    /// Publishing to the broker.
    #[serde(rename_all = "snake_case")]
    Connected {
        address: String,
    },
    /// Cannot connect at all (`no_host`: no broker set).
    #[serde(rename_all = "snake_case")]
    Error {
        message: String,
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
    /// Held across read, change, save and swap of the settings, so changes
    /// from the dashboard, the tray and the API never overwrite each other.
    pub settings_lock: parking_lot::Mutex<()>,
    /// The latest reading of each I2C channel, in channel order.
    pub latest: RwLock<Vec<Reading>>,
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

    /// Keep `r` as the latest reading of its channel.
    pub fn set_latest(&self, r: Reading) {
        let mut latest = self.latest.write();
        match latest.binary_search_by_key(&r.channel, |x| x.channel) {
            Ok(i) => latest[i] = r,
            Err(i) => latest.insert(i, r),
        }
    }
}
