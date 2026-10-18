//! The running desktop app or `tower-bridge-cli run`, reached through its
//! local API server: it holds the bridge, so commands go to it.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use anyhow::{Context, Result};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tower_bridge_core::alerts::ActiveAlert;
use tower_bridge_core::api::request;
use tower_bridge_core::gps::GpsInfo;
use tower_bridge_core::model::{DeviceState, Reading};
use tower_bridge_core::nfc::NfcInfo;
use tower_bridge_core::relay::RelayInfo;
use tower_bridge_core::store::History;
use tower_bridge_service::MqttState;
use tower_bridge_service::lock::Holder;
use tower_bridge_service::settings::Settings;

use crate::exit;

const TIMEOUT: Duration = Duration::from_secs(15);

pub struct Remote {
    addr: SocketAddr,
    token: String,
    /// The shared settings, for display preferences.
    pub settings_file: std::path::PathBuf,
    /// Who runs, for messages.
    pub holder: Holder,
}

/// What `/api/v1/status` says.
#[derive(Debug, Deserialize)]
pub struct Status {
    pub device: DeviceState,
    pub relay: Option<RelayInfo>,
    #[serde(default)]
    pub nfc: Option<NfcInfo>,
    #[serde(default)]
    pub mqtt: Option<MqttState>,
    pub sample_interval_ms: u64,
    pub active_alerts: Vec<ActiveAlert>,
}

#[derive(Deserialize)]
struct Latest {
    readings: Vec<Reading>,
}

#[derive(Deserialize)]
struct ErrorBody {
    error: ErrorDetail,
}

#[derive(Deserialize)]
struct ErrorDetail {
    code: String,
    message: String,
}

impl Remote {
    pub fn new(settings: &Settings, holder: Holder, settings_file: std::path::PathBuf) -> Self {
        Self {
            addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), settings.api.port),
            token: settings.api.token.clone(),
            settings_file,
            holder,
        }
    }

    /// Where it is reached, for messages.
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    fn call(&self, method: &str, path: &str, body: Option<Value>) -> Result<Option<Vec<u8>>> {
        let auth = format!("Bearer {}", self.token);
        let mut headers = vec![("Accept", "application/json")];
        if !self.token.is_empty() {
            headers.push(("Authorization", auth.as_str()));
        }
        let body = body.map(|b| b.to_string()).unwrap_or_default();
        if !body.is_empty() {
            headers.push(("Content-Type", "application/json"));
        }
        let r =
            request(self.addr, method, path, &headers, body.as_bytes(), TIMEOUT).map_err(|e| {
                crate::failure(
                    exit::BUSY,
                    format!(
                        "{} holds the bridge, but its API server at {} does not answer: {e}",
                        self.holder,
                        self.url()
                    ),
                    Some("check the API server in its settings (enabled, port), or stop it"),
                )
            })?;
        if (200..300).contains(&r.status) {
            return Ok(Some(r.body));
        }
        let detail = serde_json::from_slice::<ErrorBody>(&r.body)
            .map(|e| e.error)
            .unwrap_or(ErrorDetail {
                code: String::new(),
                message: String::from_utf8_lossy(&r.body).trim().to_string(),
            });
        match r.status {
            404 if detail.code.starts_with("no_") || detail.code == "not_found" => Ok(None),
            401 => Err(crate::failure(
                exit::ERROR,
                format!(
                    "{} refused the access token: {}",
                    self.holder, detail.message
                ),
                Some("the token in settings.json no longer matches; restart it"),
            )),
            403 => Err(crate::failure(
                exit::ERROR,
                format!("{} does not allow changes over its API", self.holder),
                Some(
                    "turn on Allow changes (in the app: Settings, API & MCP), or `config set api.allow_control true` while it is stopped",
                ),
            )),
            _ => Err(anyhow::anyhow!(
                "{} answered {}: {}",
                self.holder,
                r.status,
                detail.message
            )),
        }
    }

    fn get<T: DeserializeOwned>(&self, path: &str) -> Result<Option<T>> {
        match self.call("GET", path, None)? {
            Some(body) => serde_json::from_slice(&body)
                .with_context(|| format!("reading {path} from {}", self.holder))
                .map(Some),
            None => Ok(None),
        }
    }

    fn send<T: DeserializeOwned>(&self, method: &str, path: &str, body: Value) -> Result<T> {
        let body = self.call(method, path, Some(body))?.ok_or_else(|| {
            crate::failure(exit::NOT_FOUND, format!("{path}: not connected"), None)
        })?;
        serde_json::from_slice(&body).with_context(|| format!("reading the answer of {path}"))
    }

    pub fn status(&self) -> Result<Status> {
        self.get("/api/v1/status")?
            .ok_or_else(|| anyhow::anyhow!("{} has no status", self.holder))
    }

    pub fn latest(&self) -> Result<Vec<Reading>> {
        Ok(self
            .get::<Latest>("/api/v1/readings/latest")?
            .map(|l| l.readings)
            .unwrap_or_default())
    }

    pub fn gps(&self) -> Result<Option<GpsInfo>> {
        self.get("/api/v1/gps")
    }

    pub fn history(&self, from: i64, to: i64, max_points: usize) -> Result<History> {
        self.get(&format!(
            "/api/v1/readings?from={from}&to={to}&max_points={max_points}"
        ))?
        .ok_or_else(|| anyhow::anyhow!("no history"))
    }

    pub fn identify(&self) -> Result<()> {
        self.send::<Value>("POST", "/api/v1/device/identify", json!({}))
            .map(|_| ())
    }

    pub fn rescan(&self) -> Result<()> {
        self.send::<Value>("POST", "/api/v1/device/rescan", json!({}))
            .map(|_| ())
    }

    pub fn set_relay(&self, state: bool) -> Result<RelayInfo> {
        self.send("PUT", "/api/v1/relay", json!({ "state": state }))
    }

    pub fn pulse_relay(&self, state: Option<bool>, duration_ms: Option<u64>) -> Result<RelayInfo> {
        self.send(
            "POST",
            "/api/v1/relay/pulse",
            json!({ "state": state, "duration_ms": duration_ms }),
        )
    }

    pub fn write_nfc(&self, url: &str) -> Result<NfcInfo> {
        self.send("PUT", "/api/v1/nfc", json!({ "url": url }))
    }
}
