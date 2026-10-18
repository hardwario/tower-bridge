//! Starting and stopping the service: everything the desktop app does
//! without a window, and what `tower-bridge-cli run` and its terminal UI
//! run.

use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use parking_lot::{Mutex, RwLock};
use tower_bridge_core::acquisition::{self, AcqConfig, Acquisition};
use tower_bridge_core::bus::SystemClock;
use tower_bridge_core::model::DeviceState;
use tower_bridge_core::store::Store;
use tracing::{error, info, warn};

use crate::api::ApiController;
use crate::hub::{self, HubMsg};
use crate::lock::{Holder, InstanceLock};
use crate::observer::Observer;
use crate::paths::Paths;
use crate::settings::Settings;
use crate::state::{ApiState, MqttState, ServiceState};

/// How much of the service runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Everything the desktop app does: recording, alerts, MQTT and the
    /// API server, as the settings say.
    Full,
    /// A short command: the bridge only. Nothing is recorded, published or
    /// served, and the settings are only read.
    Session,
}

pub struct ServiceConfig {
    pub paths: Paths,
    pub mode: Mode,
    pub simulated: bool,
    /// Program name and version: "tower-bridge" or "tower-bridge-cli".
    pub program: String,
    pub version: String,
    pub observer: Arc<dyn Observer>,
}

#[derive(Debug, thiserror::Error)]
pub enum StartError {
    #[error("{0} is already running with the data folder {1}")]
    AlreadyRunning(Holder, std::path::PathBuf),
    #[error("cannot use the data folder {0}: {1}")]
    DataDir(std::path::PathBuf, std::io::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// The running service; stopped by `shutdown` (or when dropped, briefly).
pub struct Service {
    state: Arc<ServiceState>,
    acquisition: Mutex<Option<Acquisition>>,
    hub: Mutex<Option<JoinHandle<()>>>,
    /// Settings were created on this start.
    pub first_run: bool,
    /// Held by a full instance; `None` in a session, or where the folder
    /// cannot hold a lock file.
    _lock: Option<InstanceLock>,
}

impl std::ops::Deref for Service {
    type Target = ServiceState;

    fn deref(&self) -> &ServiceState {
        &self.state
    }
}

impl Service {
    pub fn start(config: ServiceConfig) -> Result<Self, StartError> {
        let paths = config.paths;
        paths
            .create()
            .map_err(|e| StartError::DataDir(paths.data_dir.clone(), e))?;
        let full = config.mode == Mode::Full;
        // A session takes no lock: an instance starting meanwhile waits for
        // the bridge instead of refusing to start (the app at login next to
        // a script calling `status`).
        if let Some(holder) = InstanceLock::holder(&paths.lock_file()) {
            return Err(StartError::AlreadyRunning(holder, paths.data_dir.clone()));
        }
        let lock = match full.then(|| InstanceLock::acquire(&paths.lock_file(), &config.program)) {
            None => None,
            Some(Ok(Ok(lock))) => Some(lock),
            Some(Ok(Err(holder))) => {
                return Err(StartError::AlreadyRunning(holder, paths.data_dir.clone()));
            }
            // A folder without room for the lock (read-only) still runs, as
            // before there was a lock.
            Some(Err(e)) => {
                warn!("cannot lock the data folder, not checking for another instance: {e}");
                None
            }
        };
        info!(
            "{} {} starting (data: {}{}{})",
            config.program,
            config.version,
            paths.data_dir.display(),
            if paths.portable { ", portable" } else { "" },
            if config.simulated {
                ", simulated bridge"
            } else {
                ""
            },
        );

        // A session only reads: `load` may back the file up or move it aside.
        let (settings, first_run) = if full {
            Settings::load(&paths.settings_file())
        } else {
            (Settings::read(&paths.settings_file()), false)
        };
        // Persist defaults added by newer versions (and the generated API token).
        if full && let Err(e) = settings.save(&paths.settings_file()) {
            error!("cannot write settings: {e}");
        }
        let settings = Arc::new(settings);

        let store = match full.then(|| Store::open_or_replace(&paths.db_file())) {
            None => None,
            Some(opened) => match opened {
                Ok((mut s, aside)) => {
                    if let Some(aside) = aside {
                        warn!(
                            "the database was damaged; moved to {} and started anew",
                            aside.display()
                        );
                    }
                    if config.simulated {
                        seed_simulated_history(&mut s, settings.sample_interval_ms);
                    }
                    Some(s)
                }
                Err(e) => {
                    error!("cannot open the database, history is disabled: {e}");
                    None
                }
            },
        };

        let (acq_tx, acq_rx) = crossbeam_channel::unbounded();
        let (hub_tx, hub_rx) = crossbeam_channel::unbounded();
        let (mqtt_cmd_tx, mqtt_cmd_rx) = crossbeam_channel::unbounded();
        let opener = if config.simulated {
            acquisition::simulated_opener()
        } else {
            acquisition::hardware_opener(settings.backend)
        };
        let acq = Acquisition::spawn(
            AcqConfig {
                interval: Duration::from_millis(settings.sample_interval_ms),
                channel: settings.i2c_channel,
                state_dir: Some(paths.data_dir.clone()),
            },
            opener,
            Box::new(SystemClock::new()),
            acq_tx,
        )?;

        let state = Arc::new_cyclic(|me| ServiceState {
            settings: RwLock::new(settings.clone()),
            settings_lock: Mutex::new(()),
            latest: RwLock::new(Vec::new()),
            device: RwLock::new(DeviceState::Searching),
            relay: RwLock::new(None),
            nfc: RwLock::new(None),
            gps: RwLock::new(None),
            alerts: RwLock::new(Vec::new()),
            mqtt: RwLock::new(MqttState::Disabled),
            api: RwLock::new(ApiState::Disabled),
            paths,
            mode: config.mode,
            simulated: config.simulated,
            program: config.program,
            version: config.version,
            acq_tx: acq.sender(),
            hub_tx: hub_tx.clone(),
            observer: config.observer,
            api_server: ApiController::default(),
            me: me.clone(),
        });
        state.apply_api();
        crate::control::spawn_mqtt_commands(state.clone(), mqtt_cmd_rx)?;
        let hub = hub::spawn(state.clone(), acq_rx, hub_rx, store, mqtt_cmd_tx)?;
        let _ = hub_tx.send(HubMsg::Init);
        Ok(Self {
            state,
            acquisition: Mutex::new(Some(acq)),
            hub: Mutex::new(Some(hub)),
            first_run,
            _lock: lock,
        })
    }

    pub fn state(&self) -> &Arc<ServiceState> {
        &self.state
    }

    /// Stop the threads: MQTT says goodbye, the sensors' state is saved,
    /// the LED goes off and the bridge is closed. Waits up to `timeout`.
    pub fn shutdown(&self, timeout: Duration) {
        let _ = self.state.hub_tx.send(HubMsg::Shutdown);
        self.state.stop_api();
        if let Some(acq) = self.acquisition.lock().take() {
            acq.shutdown(timeout);
        }
        if let Some(hub) = self.hub.lock().take() {
            let deadline = std::time::Instant::now() + timeout;
            while !hub.is_finished() && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        info!("stopped");
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        if self.acquisition.lock().is_some() {
            self.shutdown(Duration::from_secs(1));
        }
    }
}

/// Development aid: with `TOWER_BRIDGE_SEED_DAYS=<n>` in simulation mode,
/// fill an empty database with `n` days of generated readings.
fn seed_simulated_history(store: &mut Store, interval_ms: u64) {
    let Some(days) = std::env::var("TOWER_BRIDGE_SEED_DAYS")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|d| (1..=400).contains(d))
    else {
        return;
    };
    if !matches!(tower_bridge_core::store::first_ts(store.conn()), Ok(None)) {
        return;
    }
    let step = i64::try_from(interval_ms.max(2_000)).unwrap_or(i64::MAX);
    let now = tower_bridge_core::acquisition::unix_ms();
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut noise = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f64 / (1u64 << 53) as f64 - 0.5
    };
    let mut batch = Vec::with_capacity(10_000);
    let mut ts = now - days * 86_400_000;
    while ts < now {
        // The simulated Climate Module on channel 0 and tags on channel 1.
        for channel in [0u8, 1] {
            let n = [noise(), noise(), noise(), noise()];
            let wall = ts as f64 / 1000.0;
            let mut values = tower_bridge_core::bus::mock::simulated_values(
                wall - 1800.0 * f64::from(channel),
                n,
            );
            if channel == 1 {
                values.temperature = values.temperature.map(|t| t - 3.5);
                values.humidity = values.humidity.map(|h| (h + 9.0).min(100.0));
                values.illuminance = values.illuminance.map(|l| l * 1.8);
                values.pressure = values.pressure.map(|p| p + 12.0);
            }
            batch.push(tower_bridge_core::model::Reading {
                ts,
                device: "SIMULATED".into(),
                channel,
                values,
                altitude: None,
            });
        }
        if batch.len() == 10_000 {
            let _ = store.insert_many(&batch);
            batch.clear();
        }
        ts += step;
    }
    let _ = store.insert_many(&batch);
    info!("seeded {days} days of simulated history");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observer::Silent;

    fn config(dir: &std::path::Path, mode: Mode) -> ServiceConfig {
        ServiceConfig {
            paths: Paths {
                data_dir: dir.to_path_buf(),
                portable: false,
            },
            mode,
            simulated: true,
            program: "test".into(),
            version: "0".into(),
            observer: Arc::new(Silent),
        }
    }

    #[test]
    fn a_session_does_not_keep_the_service_out() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        // A short command runs; the app starts at login meanwhile.
        let session = Service::start(config(dir.path(), Mode::Session))?;
        let full = Service::start(config(dir.path(), Mode::Full))?;
        // Next to the service, neither a session nor a second service starts:
        // commands go through its API instead.
        assert!(matches!(
            Service::start(config(dir.path(), Mode::Session)),
            Err(StartError::AlreadyRunning(..))
        ));
        assert!(matches!(
            Service::start(config(dir.path(), Mode::Full)),
            Err(StartError::AlreadyRunning(..))
        ));
        session.shutdown(Duration::from_secs(2));
        full.shutdown(Duration::from_secs(2));
        Ok(())
    }

    #[test]
    fn a_session_only_reads_the_settings() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let file = dir.path().join("settings.json");
        std::fs::write(&file, "not json")?;
        let session = Service::start(config(dir.path(), Mode::Session))?;
        session.shutdown(Duration::from_secs(2));
        assert_eq!(
            std::fs::read_to_string(&file)?,
            "not json",
            "neither moved nor rewritten"
        );
        assert!(
            !dir.path().join("tower-bridge.db").exists(),
            "nothing recorded"
        );
        Ok(())
    }
}
