//! The service's shared state: what is measured and connected now.

use std::sync::{Arc, Weak};

use crossbeam_channel::Sender;
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use tower_bridge_core::acquisition::AcqCommand;
use tower_bridge_core::alerts::ActiveAlert;
use tower_bridge_core::gps::GpsInfo;
use tower_bridge_core::model::{DeviceState, Reading};
use tower_bridge_core::nfc::NfcInfo;
use tower_bridge_core::relay::RelayInfo;

use crate::api::ApiController;
use crate::hub::HubMsg;
use crate::observer::Observer;
use crate::paths::Paths;
use crate::service::Mode;
use crate::settings::Settings;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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

pub struct ServiceState {
    pub(crate) settings: RwLock<Arc<Settings>>,
    /// Held across read, change, save and swap of the settings, so changes
    /// from the front end, the API and MQTT never overwrite each other.
    pub(crate) settings_lock: Mutex<()>,
    /// The latest reading of each I2C channel, in channel order.
    pub latest: RwLock<Vec<Reading>>,
    pub device: RwLock<DeviceState>,
    /// Detected Relay Module, if any.
    pub relay: RwLock<Option<RelayInfo>>,
    pub nfc: RwLock<Option<NfcInfo>>,
    pub gps: RwLock<Option<GpsInfo>>,
    pub alerts: RwLock<Vec<ActiveAlert>>,
    pub mqtt: RwLock<MqttState>,
    pub api: RwLock<ApiState>,
    pub paths: Paths,
    pub mode: Mode,
    pub simulated: bool,
    /// Program name and version, for the API and the lock file.
    pub program: String,
    pub version: String,
    pub(crate) acq_tx: Sender<AcqCommand>,
    pub(crate) hub_tx: Sender<HubMsg>,
    pub(crate) observer: Arc<dyn Observer>,
    pub(crate) api_server: ApiController,
    /// This state itself, for the API server's backend.
    pub(crate) me: Weak<ServiceState>,
}

impl ServiceState {
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

    pub(crate) fn observer(&self) -> &dyn Observer {
        self.observer.as_ref()
    }
}
