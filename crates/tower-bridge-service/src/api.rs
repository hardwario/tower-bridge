//! Local API server (REST, Prometheus metrics, MCP): lifecycle and the
//! backend that serves the service's state. The server can be stopped and
//! re-bound at runtime.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Weak};

use parking_lot::Mutex;
use tower_bridge_core::acquisition::AcqError;
use tower_bridge_core::alerts::{ActiveAlert, AlertRule, AlertRules};
use tower_bridge_core::api::{ApiBackend, ApiError, ApiInfo, ApiServer, Policy, stdio};
use tower_bridge_core::model::{DeviceState, Metric, Reading};
use tower_bridge_core::nfc::{MAX_URL, NfcInfo};
use tower_bridge_core::relay::RelayInfo;
use tower_bridge_core::store::{self, AlertLogEntry, CsvOptions, History, HistoryQuery};
use tracing::warn;

use crate::paths::Paths;
use crate::service::Mode;
use crate::settings::{Settings, TemperatureUnit};
use crate::state::{ApiState, ServiceState};

/// Running server, if any, keyed by its bind address.
#[derive(Default)]
pub(crate) struct ApiController {
    server: Mutex<Option<(SocketAddr, ApiServer)>>,
}

/// Where the server listens with these settings.
pub fn bind_addr(settings: &Settings) -> SocketAddr {
    let ip = if settings.api.allow_lan {
        IpAddr::V4(Ipv4Addr::UNSPECIFIED)
    } else {
        IpAddr::V4(Ipv4Addr::LOCALHOST)
    };
    SocketAddr::new(ip, settings.api.port)
}

impl ServiceState {
    /// Start, stop or re-bind the server to match the settings. Token, MCP
    /// and control switches are read live and need no restart.
    pub(crate) fn apply_api(&self) {
        let settings = self.settings();
        let serve = settings.api.enabled && self.mode == Mode::Full;
        let desired = serve.then(|| bind_addr(&settings));
        let mut guard = self.api_server.server.lock();
        let current = guard.as_ref().map(|(addr, _)| *addr);
        if current == desired && !matches!(*self.api.read(), ApiState::Error { .. }) {
            return;
        }
        if let Some((_, mut server)) = guard.take() {
            server.stop();
        }
        let state = match desired {
            None => ApiState::Disabled,
            Some(addr) => match ApiServer::start(
                addr,
                Arc::new(Backend {
                    state: self.me.clone(),
                }),
            ) {
                Ok(server) => {
                    let port = server.local_addr().port();
                    *guard = Some((addr, server));
                    ApiState::Running {
                        address: addr.to_string(),
                        url: format!("http://127.0.0.1:{port}"),
                    }
                }
                Err(e) => {
                    warn!("API server: cannot listen on {addr}: {e}");
                    ApiState::Error {
                        port: addr.port(),
                        message: e.to_string(),
                    }
                }
            },
        };
        drop(guard);
        *self.api.write() = state.clone();
        self.observer().api(self, &state);
    }

    pub(crate) fn stop_api(&self) {
        if let Some((_, mut server)) = self.api_server.server.lock().take() {
            server.stop();
        }
    }
}

/// Serves the state; weak, so a running server does not keep it alive.
struct Backend {
    state: Weak<ServiceState>,
}

impl Backend {
    fn state(&self) -> Result<Arc<ServiceState>, ApiError> {
        self.state
            .upgrade()
            .ok_or_else(|| ApiError::Unavailable("shutting down".into()))
    }

    fn reader(&self) -> Result<store::Connection, ApiError> {
        store::open_reader(&self.state()?.paths.db_file())
            .map_err(|e| ApiError::Internal(e.to_string()))
    }
}

impl ApiBackend for Backend {
    fn info(&self) -> ApiInfo {
        let (name, version) = self
            .state
            .upgrade()
            .map(|s| (s.program.clone(), s.version.clone()))
            .unwrap_or_default();
        ApiInfo { name, version }
    }

    fn policy(&self) -> Policy {
        match self.state.upgrade() {
            Some(s) => {
                let s = s.settings();
                Policy {
                    token: s.api.token.clone(),
                    allow_control: s.api.allow_control,
                    mcp: s.api.mcp,
                }
            }
            None => Policy {
                token: String::new(),
                allow_control: false,
                mcp: false,
            },
        }
    }

    fn device(&self) -> DeviceState {
        self.state
            .upgrade()
            .map_or(DeviceState::Searching, |s| s.device.read().clone())
    }

    fn latest(&self) -> Vec<Reading> {
        self.state
            .upgrade()
            .map(|s| s.latest.read().clone())
            .unwrap_or_default()
    }

    fn sample_interval_ms(&self) -> u64 {
        self.state
            .upgrade()
            .map_or(20_000, |s| s.settings().sample_interval_ms)
    }

    fn history(&self, from: i64, to: i64, max_points: usize) -> Result<History, ApiError> {
        let conn = self.reader()?;
        let query = HistoryQuery {
            from,
            to,
            max_points,
            sample_interval_ms: i64::try_from(self.sample_interval_ms()).unwrap_or(i64::MAX),
        };
        store::history(&conn, &query).map_err(|e| ApiError::Internal(e.to_string()))
    }

    fn csv(&self, from: i64, to: i64, out: &mut dyn std::io::Write) -> Result<(), ApiError> {
        let conn = self.reader()?;
        let settings = self.state()?.settings();
        let opts = CsvOptions {
            fahrenheit: settings.temperature_unit == TemperatureUnit::Fahrenheit,
            sea_level_pa: settings.sea_level_pressure_pa,
        };
        store::write_csv(&conn, from, to, opts, out, |_, _| true)
            .map(|_| ())
            .map_err(|e| ApiError::Internal(e.to_string()))
    }

    fn alert_rules(&self) -> AlertRules {
        self.state
            .upgrade()
            .map(|s| s.settings().alerts.clone())
            .unwrap_or_default()
    }

    fn active_alerts(&self) -> Vec<ActiveAlert> {
        self.state
            .upgrade()
            .map(|s| s.alerts.read().clone())
            .unwrap_or_default()
    }

    fn alert_log(&self, limit: usize) -> Result<Vec<AlertLogEntry>, ApiError> {
        let conn = self.reader()?;
        store::alert_log(&conn, limit).map_err(|e| ApiError::Internal(e.to_string()))
    }

    fn set_alert_rule(&self, metric: Metric, rule: AlertRule) -> Result<AlertRules, ApiError> {
        let saved = self
            .state()?
            .update_settings(|settings| *settings.alerts.get_mut(metric) = rule)
            .map_err(|e| ApiError::Internal(format!("cannot save settings: {e}")))?;
        Ok(saved.alerts.clone())
    }

    fn identify(&self) -> Result<(), ApiError> {
        self.state()?.identify().map_err(api_error)
    }

    fn rescan(&self) -> Result<(), ApiError> {
        self.state()?.rescan().map(|_| ()).map_err(api_error)
    }

    fn relay(&self) -> Option<RelayInfo> {
        self.state.upgrade().and_then(|s| *s.relay.read())
    }

    fn gps(&self) -> Option<tower_bridge_core::gps::GpsInfo> {
        self.state.upgrade().and_then(|s| s.gps.read().clone())
    }

    fn nfc(&self) -> Option<NfcInfo> {
        self.state.upgrade().and_then(|s| s.nfc.read().clone())
    }

    fn write_nfc(&self, url: String) -> Result<NfcInfo, ApiError> {
        if url.len() > MAX_URL {
            return Err(ApiError::BadRequest(format!(
                "the URL is longer than {MAX_URL} bytes"
            )));
        }
        self.state()?.write_nfc_url(url).map_err(api_error)
    }

    fn mqtt(&self) -> serde_json::Value {
        self.state
            .upgrade()
            .and_then(|s| serde_json::to_value(&*s.mqtt.read()).ok())
            .unwrap_or_default()
    }

    fn set_relay(&self, state: bool) -> Result<RelayInfo, ApiError> {
        self.state()?.set_relay(state).map_err(api_error)
    }

    fn pulse_relay(
        &self,
        state: Option<bool>,
        duration: Option<std::time::Duration>,
    ) -> Result<RelayInfo, ApiError> {
        self.state()?
            .pulse_relay(state, duration)
            .map_err(api_error)
    }
}

fn api_error(e: AcqError) -> ApiError {
    match e {
        AcqError::NotConnected | AcqError::NoRelay | AcqError::NoNfc => {
            ApiError::Unavailable(e.to_string())
        }
        AcqError::Nfc(m) | AcqError::Bus(m) => ApiError::Internal(m),
    }
}

/// MCP over stdin/stdout, forwarding to the running instance's API server
/// (`tower-bridge --mcp-stdio`, `tower-bridge-cli mcp`).
pub fn mcp_stdio(paths: &Paths, info: ApiInfo) -> Result<(), String> {
    let settings_file = paths.settings_file();
    let target = move || {
        // Read only: the running instance owns the file.
        let settings = Settings::read(&settings_file);
        stdio::StdioTarget {
            addr: settings
                .api
                .enabled
                .then(|| SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), settings.api.port)),
            token: settings.api.token,
            allow_control: settings.api.allow_control,
        }
    };
    stdio::run(info, target).map_err(|e| e.to_string())
}
