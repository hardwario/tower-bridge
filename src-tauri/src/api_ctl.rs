//! Local API server (REST, Prometheus metrics, MCP): lifecycle and the
//! backend that serves the app's state. Unlike the MQTT broker, the server
//! can be stopped and re-bound at runtime.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use parking_lot::Mutex;
use tauri::{AppHandle, Emitter, Manager};
use tower_bridge_core::alerts::{ActiveAlert, AlertRule, AlertRules};
use tower_bridge_core::api::{ApiBackend, ApiError, ApiInfo, ApiServer, Policy, stdio};
use tower_bridge_core::model::{DeviceState, Metric, Reading};
use tower_bridge_core::relay::RelayInfo;
use tower_bridge_core::store::{self, AlertLogEntry, CsvOptions, History, HistoryQuery};
use tracing::warn;

use crate::paths::Paths;
use crate::settings::{Settings, TemperatureUnit};
use crate::state::{ApiState, AppState};

/// Running server, if any, keyed by its bind address.
#[derive(Default)]
pub struct ApiController {
    server: Mutex<Option<(SocketAddr, ApiServer)>>,
}

fn bind_addr(settings: &Settings) -> SocketAddr {
    let ip = if settings.api.allow_lan {
        IpAddr::V4(Ipv4Addr::UNSPECIFIED)
    } else {
        IpAddr::V4(Ipv4Addr::LOCALHOST)
    };
    SocketAddr::new(ip, settings.api.port)
}

/// Start, stop or re-bind the server to match the settings. Token, MCP and
/// control switches are read live and need no restart.
pub fn apply(app: &AppHandle, settings: &Settings) {
    let Some(ctl) = app.try_state::<ApiController>() else {
        return;
    };
    let desired = settings.api.enabled.then(|| bind_addr(settings));
    let mut guard = ctl.server.lock();
    let current = guard.as_ref().map(|(addr, _)| *addr);
    let state_unchanged = app
        .try_state::<AppState>()
        .is_some_and(|s| !matches!(*s.api.read(), ApiState::Error { .. }));
    if current == desired && state_unchanged {
        return;
    }
    if let Some((_, mut server)) = guard.take() {
        server.stop();
    }
    let state = match desired {
        None => ApiState::Disabled,
        Some(addr) => match ApiServer::start(addr, Arc::new(AppApiBackend { app: app.clone() })) {
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
    if let Some(s) = app.try_state::<AppState>() {
        *s.api.write() = state.clone();
    }
    let _ = app.emit("api-state", &state);
}

pub fn stop(app: &AppHandle) {
    if let Some(ctl) = app.try_state::<ApiController>()
        && let Some((_, mut server)) = ctl.server.lock().take()
    {
        server.stop();
    }
}

struct AppApiBackend {
    app: AppHandle,
}

impl AppApiBackend {
    fn state(&self) -> tauri::State<'_, AppState> {
        self.app.state::<AppState>()
    }

    fn reader(&self) -> Result<store::Connection, ApiError> {
        store::open_reader(&self.state().paths.db_file())
            .map_err(|e| ApiError::Internal(e.to_string()))
    }
}

impl ApiBackend for AppApiBackend {
    fn info(&self) -> ApiInfo {
        ApiInfo {
            name: "tower-bridge".into(),
            version: self.app.package_info().version.to_string(),
        }
    }

    fn policy(&self) -> Policy {
        let s = self.state().settings();
        Policy {
            token: s.api.token.clone(),
            allow_control: s.api.allow_control,
            mcp: s.api.mcp,
        }
    }

    fn device(&self) -> DeviceState {
        self.state().device.read().clone()
    }

    fn latest(&self) -> Vec<Reading> {
        self.state().latest.read().clone()
    }

    fn sample_interval_s(&self) -> u64 {
        self.state().settings().sample_interval_s
    }

    fn history(&self, from: i64, to: i64, max_points: usize) -> Result<History, ApiError> {
        let conn = self.reader()?;
        let query = HistoryQuery {
            from,
            to,
            max_points,
            sample_interval_ms: self.sample_interval_s() as i64 * 1000,
        };
        store::history(&conn, &query).map_err(|e| ApiError::Internal(e.to_string()))
    }

    fn csv(&self, from: i64, to: i64) -> Result<Vec<u8>, ApiError> {
        let conn = self.reader()?;
        let settings = self.state().settings();
        let opts = CsvOptions {
            fahrenheit: settings.temperature_unit == TemperatureUnit::Fahrenheit,
            sea_level_pa: settings.sea_level_pressure_pa,
        };
        let mut out = Vec::new();
        store::write_csv(&conn, from, to, opts, &mut out, |_, _| true)
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        Ok(out)
    }

    fn alert_rules(&self) -> AlertRules {
        self.state().settings().alerts.clone()
    }

    fn active_alerts(&self) -> Vec<ActiveAlert> {
        self.state().alerts.read().clone()
    }

    fn alert_log(&self, limit: usize) -> Result<Vec<AlertLogEntry>, ApiError> {
        let conn = self.reader()?;
        store::alert_log(&conn, limit).map_err(|e| ApiError::Internal(e.to_string()))
    }

    fn set_alert_rule(&self, metric: Metric, rule: AlertRule) -> Result<AlertRules, ApiError> {
        let saved = crate::actions::update_settings(&self.app, |settings| {
            *settings.alerts.get_mut(metric) = rule;
        })
        .map_err(|e| ApiError::Internal(format!("cannot save settings: {e}")))?;
        Ok(saved.alerts.clone())
    }

    fn identify(&self) -> Result<(), ApiError> {
        crate::actions::identify(&self.app).map_err(api_error)
    }

    fn rescan(&self) -> Result<(), ApiError> {
        crate::actions::rescan(&self.app)
            .map(|_| ())
            .map_err(api_error)
    }

    fn relay(&self) -> Option<RelayInfo> {
        *self.app.state::<AppState>().relay.read()
    }

    fn gps(&self) -> Option<tower_bridge_core::gps::GpsInfo> {
        self.app.state::<AppState>().gps.read().clone()
    }

    fn set_relay(&self, state: bool) -> Result<RelayInfo, ApiError> {
        crate::actions::set_relay(&self.app, state).map_err(api_error)
    }

    fn pulse_relay(
        &self,
        state: Option<bool>,
        duration: Option<std::time::Duration>,
    ) -> Result<RelayInfo, ApiError> {
        crate::actions::pulse_relay(&self.app, state, duration).map_err(api_error)
    }
}

fn api_error(e: tower_bridge_core::acquisition::AcqError) -> ApiError {
    match e {
        tower_bridge_core::acquisition::AcqError::NotConnected
        | tower_bridge_core::acquisition::AcqError::NoRelay
        | tower_bridge_core::acquisition::AcqError::NoNfc => ApiError::Unavailable(e.to_string()),
        tower_bridge_core::acquisition::AcqError::Nfc(m)
        | tower_bridge_core::acquisition::AcqError::Bus(m) => ApiError::Internal(m),
    }
}

/// `tower-bridge --mcp-stdio`: MCP over stdin/stdout, forwarding to the
/// running app. Runs without starting the GUI.
pub fn run_mcp_stdio(simulated: bool) -> i32 {
    let Some(paths) = Paths::resolve_standalone(simulated) else {
        eprintln!("tower-bridge: cannot determine the data folder");
        return 1;
    };
    let settings_file = paths.settings_file();
    let info = ApiInfo {
        name: "tower-bridge".into(),
        version: env!("CARGO_PKG_VERSION").into(),
    };
    let target = move || {
        let (settings, _) = Settings::load(&settings_file);
        stdio::StdioTarget {
            addr: settings
                .api
                .enabled
                .then(|| SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), settings.api.port)),
            token: settings.api.token,
            allow_control: settings.api.allow_control,
        }
    };
    match stdio::run(info, target) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("tower-bridge: {e}");
            1
        }
    }
}
