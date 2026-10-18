//! Optional local HTTP API: REST endpoints, Prometheus metrics and an MCP
//! server (Streamable HTTP at `/mcp`, plus a stdio bridge for clients that
//! only launch local processes).
//!
//! The server is transport only; the application supplies data and actions
//! through [`ApiBackend`].
//!
//! Access rules:
//! - Requests carrying the valid bearer token are always accepted.
//! - Without a token, requests are accepted only when the server listens on
//!   loopback and both `Host` and `Origin` (if present) are loopback names,
//!   which blocks DNS-rebinding attacks from web pages.
//! - Changing actions additionally require [`Policy::allow_control`].

mod http_client;
pub mod mcp;
mod openapi;
mod rest;
mod security;
mod server;
pub mod stdio;

pub use http_client::{HttpResponse, request};
pub use server::ApiServer;

use std::time::Duration;

use serde::Serialize;

use crate::alerts::{ActiveAlert, AlertRule, AlertRules};
use crate::model::{DeviceState, Metric, Reading};
use crate::relay::RelayInfo;
use crate::store::{AlertLogEntry, History};

/// Live access settings, read on every request.
#[derive(Clone, Debug, Default)]
pub struct Policy {
    /// Bearer token; empty disables token authentication.
    pub token: String,
    /// Allow changing alert limits and other actions.
    pub allow_control: bool,
    /// Serve the MCP endpoint.
    pub mcp: bool,
}

/// Identity reported by the API and the MCP server.
#[derive(Clone, Debug, Serialize)]
pub struct ApiInfo {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ApiError {
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    Forbidden(String),
    #[error("{0}")]
    Unavailable(String),
    #[error("{0}")]
    Internal(String),
}

/// Data and actions provided by the application.
///
/// Methods may block (database access); the server calls them from a
/// blocking thread pool.
pub trait ApiBackend: Send + Sync + 'static {
    fn info(&self) -> ApiInfo;
    fn policy(&self) -> Policy;
    fn device(&self) -> DeviceState;
    /// The latest reading of each I2C channel, in channel order.
    fn latest(&self) -> Vec<Reading>;
    fn sample_interval_ms(&self) -> u64;
    fn history(&self, from: i64, to: i64, max_points: usize) -> Result<History, ApiError>;
    /// CSV export of individual readings in `[from, to)`.
    /// Write the readings of the range as CSV into `out`, row by row.
    fn csv(&self, from: i64, to: i64, out: &mut dyn std::io::Write) -> Result<(), ApiError>;
    fn alert_rules(&self) -> AlertRules;
    fn active_alerts(&self) -> Vec<ActiveAlert>;
    fn alert_log(&self, limit: usize) -> Result<Vec<AlertLogEntry>, ApiError>;
    /// Replace the rule for one metric; returns all rules.
    fn set_alert_rule(&self, metric: Metric, rule: AlertRule) -> Result<AlertRules, ApiError>;
    /// Blink the bridge LED.
    fn identify(&self) -> Result<(), ApiError>;
    /// Look for the sensors again.
    fn rescan(&self) -> Result<(), ApiError>;
    /// The Relay Module, when one is connected.
    fn relay(&self) -> Option<RelayInfo>;
    /// What the GPS Module knows, when one is connected.
    fn gps(&self) -> Option<crate::gps::GpsInfo> {
        None
    }
    /// The NFC Tag and its URL, when one is connected.
    fn nfc(&self) -> Option<crate::nfc::NfcInfo> {
        None
    }
    /// Write a URL to the NFC Tag; returns what it holds afterwards.
    fn write_nfc(&self, url: String) -> Result<crate::nfc::NfcInfo, ApiError> {
        let _ = url;
        Err(ApiError::Unavailable("no NFC Tag is connected".into()))
    }
    /// The connection to the MQTT broker, as the front ends show it
    /// (`{"status": "connected", "address": ...}`); `null` when unknown.
    fn mqtt(&self) -> serde_json::Value {
        serde_json::Value::Null
    }
    /// Switch the relay to `true` or `false`.
    fn set_relay(&self, state: bool) -> Result<RelayInfo, ApiError>;
    /// Timed pulse; `None` fields take the app's defaults (opposite of the
    /// current state, configured length).
    fn pulse_relay(
        &self,
        state: Option<bool>,
        duration: Option<Duration>,
    ) -> Result<RelayInfo, ApiError>;
}

/// Parse a time parameter: Unix milliseconds or an RFC 3339 timestamp.
pub fn parse_time(s: &str) -> Option<i64> {
    let s = s.trim();
    if let Ok(ms) = s.parse::<i64>() {
        return Some(ms);
    }
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|t| t.timestamp_millis())
}

/// RFC 3339 local time for a Unix millisecond timestamp.
pub fn format_time(ms: i64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_millis_opt(ms)
        .single()
        .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, false))
        .unwrap_or_default()
}

pub fn metric_from_str(s: &str) -> Option<Metric> {
    Metric::ALL.into_iter().find(|m| m.as_str() == s)
}

#[cfg(test)]
pub(crate) mod testing {
    use std::sync::Mutex;

    use super::*;
    use crate::alerts::Bound;
    use crate::model::{Backend, DeviceInfo, ModuleKind, SensorSource, Values};
    use crate::store::Series;

    /// Backend with fixed data for tests.
    pub struct FakeBackend {
        pub policy: Mutex<Policy>,
        pub rules: Mutex<AlertRules>,
        pub identified: Mutex<u32>,
        pub relay: Mutex<Option<RelayInfo>>,
        /// Last pulse request.
        pub pulsed: Mutex<Option<(Option<bool>, Option<Duration>)>>,
        /// How long a rescan takes.
        pub rescan_time: Mutex<Duration>,
        pub nfc: Mutex<Option<crate::nfc::NfcInfo>>,
    }

    impl FakeBackend {
        pub fn new(policy: Policy) -> Self {
            Self {
                policy: Mutex::new(policy),
                rules: Mutex::new(AlertRules::default()),
                identified: Mutex::new(0),
                relay: Mutex::new(Some(RelayInfo {
                    channel: 0,
                    address: 0x3B,
                    state: None,
                    pulse_until: None,
                })),
                pulsed: Mutex::new(None),
                rescan_time: Mutex::new(Duration::ZERO),
                nfc: Mutex::new(Some(crate::nfc::NfcInfo {
                    channel: 1,
                    address: 0x08,
                    part: "NT3H2111".into(),
                    url: None,
                    formatted: true,
                })),
            }
        }

        fn update_relay(&self, f: impl FnOnce(&mut RelayInfo)) -> Result<RelayInfo, ApiError> {
            let mut relay = self
                .relay
                .lock()
                .map_err(|_| ApiError::Internal("poisoned".into()))?;
            let r = relay
                .as_mut()
                .ok_or_else(|| ApiError::Unavailable("Relay Module not found".into()))?;
            f(r);
            Ok(*r)
        }
    }

    pub const NOW: i64 = 1_790_000_000_000;

    impl ApiBackend for FakeBackend {
        fn info(&self) -> ApiInfo {
            ApiInfo {
                name: "tower-bridge".into(),
                version: "0.0.0-test".into(),
            }
        }
        fn policy(&self) -> Policy {
            self.policy.lock().map(|p| p.clone()).unwrap_or_default()
        }
        fn device(&self) -> DeviceState {
            DeviceState::Ready {
                device: DeviceInfo {
                    backend: Backend::Hid,
                    serial: "SN1".into(),
                    path: "fake".into(),
                    chip: None,
                    bridges_connected: 1,
                },
                sources: [
                    (Metric::Temperature, "SHT30", 0x45),
                    (Metric::Humidity, "SHT30", 0x45),
                    (Metric::Illuminance, "OPT3001", 0x44),
                    (Metric::Pressure, "MPL3115A2", 0x60),
                ]
                .into_iter()
                .map(|(metric, sensor, address)| SensorSource {
                    metric,
                    channel: 0,
                    module: ModuleKind::ClimateModuleR2,
                    sensor: sensor.into(),
                    address,
                })
                .collect(),
            }
        }
        fn latest(&self) -> Vec<Reading> {
            vec![Reading {
                ts: NOW,
                device: "SN1".into(),
                channel: 0,
                values: Values {
                    temperature: Some(23.456),
                    humidity: Some(41.25),
                    illuminance: Some(320.4),
                    pressure: Some(100_322.0),
                    co2: None,
                    voc: None,
                },
                altitude: Some(83.2),
            }]
        }
        fn sample_interval_ms(&self) -> u64 {
            10_000
        }
        fn history(&self, from: i64, to: i64, _max: usize) -> Result<History, ApiError> {
            let series = |a: f64| Series {
                avg: vec![Some(a), None, Some(a + 1.0)],
                min: vec![Some(a - 0.5), None, Some(a + 0.5)],
                max: vec![Some(a + 0.5), None, Some(a + 1.5)],
            };
            Ok(History {
                from,
                to,
                bucket_ms: 60_000,
                t: vec![
                    (from / 1000) as f64,
                    (from / 1000 + 60) as f64,
                    (from / 1000 + 120) as f64,
                ],
                channels: vec![0],
                temperature: vec![series(22.0)],
                humidity: vec![series(40.0)],
                illuminance: vec![series(300.0)],
                pressure: vec![series(100_000.0)],
                co2: Vec::new(),
                voc: Vec::new(),
            })
        }
        fn csv(&self, _from: i64, _to: i64, out: &mut dyn std::io::Write) -> Result<(), ApiError> {
            out.write_all(b"timestamp,unix_ms\n")
                .map_err(|e| ApiError::Internal(e.to_string()))
        }
        fn alert_rules(&self) -> AlertRules {
            self.rules.lock().map(|r| r.clone()).unwrap_or_default()
        }
        fn active_alerts(&self) -> Vec<ActiveAlert> {
            vec![ActiveAlert {
                metric: Metric::Temperature,
                channel: 0,
                bound: Bound::High,
                since: NOW - 60_000,
                threshold: 23.0,
                value: 23.456,
            }]
        }
        fn alert_log(&self, _limit: usize) -> Result<Vec<AlertLogEntry>, ApiError> {
            Ok(vec![AlertLogEntry {
                ts: NOW - 60_000,
                metric: "temperature".into(),
                channel: 0,
                bound: "high".into(),
                kind: "triggered".into(),
                value: 23.4,
                threshold: 23.0,
            }])
        }
        fn set_alert_rule(&self, metric: Metric, rule: AlertRule) -> Result<AlertRules, ApiError> {
            let mut rules = self
                .rules
                .lock()
                .map_err(|_| ApiError::Internal("poisoned".into()))?;
            *rules.get_mut(metric) = rule;
            Ok(rules.clone())
        }
        fn identify(&self) -> Result<(), ApiError> {
            if let Ok(mut n) = self.identified.lock() {
                *n += 1;
            }
            Ok(())
        }
        fn rescan(&self) -> Result<(), ApiError> {
            let wait = self.rescan_time.lock().map(|d| *d).unwrap_or_default();
            std::thread::sleep(wait);
            Ok(())
        }
        fn relay(&self) -> Option<RelayInfo> {
            self.relay.lock().ok().and_then(|r| *r)
        }
        fn nfc(&self) -> Option<crate::nfc::NfcInfo> {
            self.nfc.lock().ok().and_then(|n| n.clone())
        }
        fn write_nfc(&self, url: String) -> Result<crate::nfc::NfcInfo, ApiError> {
            let mut nfc = self
                .nfc
                .lock()
                .map_err(|_| ApiError::Internal("poisoned".into()))?;
            let tag = nfc
                .as_mut()
                .ok_or_else(|| ApiError::Unavailable("no NFC Tag".into()))?;
            tag.url = Some(url);
            Ok(tag.clone())
        }
        fn mqtt(&self) -> serde_json::Value {
            serde_json::json!({ "status": "disabled" })
        }
        fn set_relay(&self, state: bool) -> Result<RelayInfo, ApiError> {
            self.update_relay(|r| {
                r.state = Some(state);
                r.pulse_until = None;
            })
        }
        fn pulse_relay(
            &self,
            state: Option<bool>,
            duration: Option<Duration>,
        ) -> Result<RelayInfo, ApiError> {
            if let Ok(mut p) = self.pulsed.lock() {
                *p = Some((state, duration));
            }
            self.update_relay(|r| {
                r.state = Some(state.unwrap_or(false));
                r.pulse_until = Some(NOW + 2000);
            })
        }
    }
}
