//! MCP (Model Context Protocol) server.
//!
//! Dual-era: supports the stateless protocol (`2026-07-28`, per-request
//! `_meta` and `server/discover`) and the legacy `initialize` handshake
//! (`2025-11-25` and earlier). The same dispatcher serves the Streamable
//! HTTP endpoint and the stdio bridge.
//!
//! Tool results use friendly units: °C, %, lx and hPa.

use std::sync::Arc;

use serde_json::{Map, Value, json};

use super::{ApiBackend, ApiError, ApiInfo, format_time, metric_from_str, parse_time};
use crate::alerts::{AlertRule, Bound};
use crate::model::{DeviceErrorKind, DeviceState, Metric};
use crate::relay::{RelayInfo, pulse_from_ms};

pub const MODERN_VERSIONS: &[&str] = &["2026-07-28"];
pub const LEGACY_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
pub const META_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
pub const META_CLIENT_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";
pub const META_SERVER_INFO: &str = "io.modelcontextprotocol/serverInfo";

pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const INTERNAL_ERROR: i64 = -32603;
pub const HEADER_MISMATCH: i64 = -32020;
pub const UNSUPPORTED_PROTOCOL_VERSION: i64 = -32022;

const INSTRUCTIONS: &str = "Indoor climate readings from a HARDWARIO TOWER Climate Module connected to this \
computer through a TOWER Bridge Module: temperature (°C), relative humidity (%), illuminance (lx) and \
barometric pressure (hPa). Use get_current_readings for the present state, get_history for trends and \
statistics over a time range, and get_alerts for configured limits and recent alerts.";

#[derive(Debug, Clone, PartialEq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    pub data: Option<Value>,
}

impl RpcError {
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    fn to_json(&self) -> Value {
        let mut e = json!({ "code": self.code, "message": self.message });
        if let Some(d) = &self.data {
            e["data"] = d.clone();
        }
        e
    }
}

/// Result of handling one message: an HTTP status hint and an optional body.
#[derive(Debug, Clone, PartialEq)]
pub struct Reply {
    pub http_status: u16,
    pub body: Option<Value>,
}

pub fn error_response(id: Value, err: &RpcError) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": err.to_json() })
}

fn server_info(info: &ApiInfo) -> Value {
    json!({ "name": info.name, "title": "TOWER Bridge", "version": info.version })
}

fn all_versions() -> Vec<&'static str> {
    MODERN_VERSIONS
        .iter()
        .chain(LEGACY_VERSIONS)
        .copied()
        .collect()
}

/// Legacy `initialize` result with version negotiation.
pub fn initialize_result(info: &ApiInfo, requested: Option<&str>) -> Value {
    let version = requested
        .filter(|v| LEGACY_VERSIONS.contains(v))
        .unwrap_or(LEGACY_VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": server_info(info),
        "instructions": INSTRUCTIONS,
    })
}

/// `server/discover` result.
pub fn discover_result(info: &ApiInfo) -> Value {
    json!({
        "resultType": "complete",
        "supportedVersions": all_versions(),
        "capabilities": { "tools": { "listChanged": false } },
        "_meta": { META_SERVER_INFO: server_info(info) },
        "instructions": INSTRUCTIONS,
        "ttlMs": 300_000,
        "cacheScope": "private",
    })
}

fn empty_schema() -> Value {
    json!({ "type": "object", "properties": {}, "additionalProperties": false })
}

fn metric_enum() -> Value {
    json!(["temperature", "humidity", "illuminance", "pressure"])
}

/// Tool definitions; changing tools are listed only when allowed.
pub fn tool_definitions(allow_control: bool) -> Vec<Value> {
    let read_only = json!({ "readOnlyHint": true, "openWorldHint": false });
    let mut tools = vec![
        json!({
            "name": "get_current_readings",
            "title": "Current readings",
            "description": "Latest temperature (°C), relative humidity (%), illuminance (lx), barometric \
        pressure (hPa) and estimated altitude (m), with the measurement time and any active alerts.",
            "inputSchema": empty_schema(),
            "annotations": read_only,
        }),
        json!({
            "name": "get_history",
            "title": "Reading history",
            "description": "Averaged readings over a time range plus min/avg/max statistics. Use 'hours' for \
        a range ending now, or 'from'/'to' for an absolute range.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "hours": { "type": "number", "minimum": 0.05, "maximum": 9600,
                        "description": "Hours back from now (default 24). Ignored when 'from' is set." },
                    "from": { "type": "string", "description": "Start, RFC 3339 (e.g. 2026-10-16T08:00:00+02:00)." },
                    "to": { "type": "string", "description": "End, RFC 3339 (default now)." },
                    "metrics": { "type": "array", "items": { "type": "string", "enum": metric_enum() },
                        "description": "Metrics to include (default all)." },
                    "max_points": { "type": "integer", "minimum": 2, "maximum": 500,
                        "description": "Approximate number of averaged points (default 48)." }
                },
                "additionalProperties": false
            },
            "annotations": read_only,
        }),
        json!({
            "name": "get_alerts",
            "title": "Alert limits and recent alerts",
            "description": "Configured alert limits per metric, currently active alerts and the most recent \
        alert events.",
            "inputSchema": empty_schema(),
            "annotations": read_only,
        }),
        json!({
            "name": "get_device_status",
            "title": "Device status",
            "description": "Connection state of the TOWER Bridge Module and Climate Module, serial number, \
        module revision, measurement interval and the Relay Module state (true, false, or null when \
        unknown) when one \
        is connected.",
            "inputSchema": empty_schema(),
            "annotations": read_only,
        }),
    ];
    if allow_control {
        tools.push(json!({
            "name": "set_alert_threshold",
            "title": "Set alert limits",
            "description": "Change the alert limits for one metric. Units: temperature °C, humidity %, \
illuminance lx, pressure hPa. Omitted fields keep their current value; null removes a limit.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "metric": { "type": "string", "enum": metric_enum() },
                    "enabled": { "type": "boolean", "description": "Turn alerts for this metric on or off." },
                    "below": { "type": ["number", "null"], "description": "Alert when the value drops below this." },
                    "above": { "type": ["number", "null"], "description": "Alert when the value rises above this." },
                    "hysteresis": { "type": "number", "minimum": 0,
                        "description": "How far the value must return inside the limit to clear the alert." },
                    "repeat_after_minutes": { "type": "number", "minimum": 1,
                        "description": "Minimum time between two notifications for the same limit." },
                    "notify_recovery": { "type": "boolean", "description": "Notify when the value is back to normal." }
                },
                "required": ["metric"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false },
        }));
        tools.push(json!({
            "name": "set_relay",
            "title": "Switch the relay",
            "description": "Switch the TOWER Relay Module (a latching changeover relay) to true or false. \
Stops a running pulse. Fails when no Relay Module is connected.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "state": { "type": "boolean" }
                },
                "required": ["state"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": true },
        }));
        tools.push(json!({
            "name": "pulse_relay",
            "title": "Pulse the relay",
            "description": "Switch the TOWER Relay Module to a state for a while, then back to the other one \
(like pressing a button). Without a state it switches to the opposite of the current one (false when unknown). \
Returns once the relay has switched over.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "state": { "type": "boolean",
                        "description": "State during the pulse." },
                    "duration_ms": { "type": "integer", "minimum": 500, "maximum": 3600000,
                        "description": "Length of the pulse in milliseconds (default: the app setting, 2000 unless changed)." }
                },
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "idempotentHint": false, "openWorldHint": true },
        }));
        tools.push(json!({
            "name": "identify_device",
            "title": "Identify the bridge",
            "description": "Blink the LED on the TOWER Bridge Module so the user can find it.",
            "inputSchema": empty_schema(),
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false },
        }));
    }
    tools
}

/// Protocol era of a request, derived from `params._meta`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Era {
    Modern,
    Legacy,
}

pub fn request_era(params: &Value) -> Result<Era, RpcError> {
    let meta = params.get("_meta");
    let Some(version) = meta
        .and_then(|m| m.get(META_VERSION))
        .and_then(Value::as_str)
    else {
        return Ok(Era::Legacy);
    };
    if !MODERN_VERSIONS.contains(&version) {
        return Err(RpcError {
            code: UNSUPPORTED_PROTOCOL_VERSION,
            message: "Unsupported protocol version".into(),
            data: Some(json!({ "supported": all_versions(), "requested": version })),
        });
    }
    if meta.and_then(|m| m.get(META_CLIENT_CAPABILITIES)).is_none() {
        return Err(RpcError::new(
            INVALID_PARAMS,
            format!("missing _meta field {META_CLIENT_CAPABILITIES}"),
        ));
    }
    Ok(Era::Modern)
}

fn http_status(era: &Era, code: i64) -> u16 {
    if *era == Era::Legacy {
        return 200;
    }
    match code {
        METHOD_NOT_FOUND => 404,
        INTERNAL_ERROR => 500,
        _ => 400,
    }
}

fn round(v: f64, decimals: i32) -> f64 {
    let f = 10f64.powi(decimals);
    (v * f).round() / f
}

fn friendly_key(m: Metric) -> &'static str {
    match m {
        Metric::Temperature => "temperature_c",
        Metric::Humidity => "humidity_pct",
        Metric::Illuminance => "illuminance_lx",
        Metric::Pressure => "pressure_hpa",
    }
}

fn friendly_unit(m: Metric) -> &'static str {
    match m {
        Metric::Temperature => "°C",
        Metric::Humidity => "%",
        Metric::Illuminance => "lx",
        Metric::Pressure => "hPa",
    }
}

/// SI value to friendly unit.
fn friendly(m: Metric, v: f64) -> f64 {
    match m {
        Metric::Temperature => round(v, 2),
        Metric::Humidity | Metric::Illuminance => round(v, 1),
        Metric::Pressure => round(v / 100.0, 2),
    }
}

/// Friendly unit (absolute value or difference) back to SI.
fn from_friendly(m: Metric, v: f64) -> f64 {
    if m == Metric::Pressure { v * 100.0 } else { v }
}

fn device_status(d: &DeviceState) -> (&'static str, Option<String>) {
    match d {
        DeviceState::Ready { .. } => ("ready", None),
        DeviceState::Searching => ("searching", None),
        DeviceState::NoModule { .. } => (
            "no_climate_module",
            Some("The TOWER Bridge Module is connected but no Climate Module answers.".into()),
        ),
        DeviceState::Error { kind, message } => (
            match kind {
                DeviceErrorKind::NotFound => "not_connected",
                DeviceErrorKind::PermissionDenied => "permission_denied",
                DeviceErrorKind::I2cDevNotLoaded => "i2c_dev_not_loaded",
                DeviceErrorKind::Busy => "busy",
                DeviceErrorKind::Failed => "error",
            },
            Some(message.clone()),
        ),
    }
}

fn relay_json(r: &RelayInfo) -> Value {
    let mut v = json!({
        "state": r.state,
        "i2c_channel": r.channel,
        "i2c_address": format!("0x{:02X}", r.address),
    });
    if let Some(until) = r.pulse_until {
        v["pulse_ends"] = json!(format_time(until));
        v["returns_to"] = json!(r.state.map(|s| !s));
    }
    v
}

fn now_ms() -> i64 {
    crate::acquisition::unix_ms()
}

fn api_err(e: ApiError) -> String {
    e.to_string()
}

pub struct McpServer {
    backend: Arc<dyn ApiBackend>,
}

impl McpServer {
    pub fn new(backend: Arc<dyn ApiBackend>) -> Self {
        Self { backend }
    }

    /// Handle one JSON-RPC message (request, notification or response).
    pub fn handle(&self, msg: &Value) -> Reply {
        let Some(obj) = msg.as_object() else {
            return Reply {
                http_status: 400,
                body: Some(error_response(
                    Value::Null,
                    &RpcError::new(INVALID_REQUEST, "expected a JSON-RPC object"),
                )),
            };
        };
        let id = obj.get("id").cloned();
        if obj.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return Reply {
                http_status: 400,
                body: Some(error_response(
                    id.unwrap_or(Value::Null),
                    &RpcError::new(INVALID_REQUEST, "jsonrpc must be \"2.0\""),
                )),
            };
        }
        let Some(method) = obj.get("method").and_then(Value::as_str) else {
            // A response to a server request; this server sends none.
            return Reply {
                http_status: 202,
                body: None,
            };
        };
        let Some(id) = id else {
            // Notifications (e.g. notifications/initialized) need no answer.
            return Reply {
                http_status: 202,
                body: None,
            };
        };
        let params = obj.get("params").cloned().unwrap_or_else(|| json!({}));
        let era = match request_era(&params) {
            Ok(era) => era,
            Err(e) => {
                return Reply {
                    http_status: 400,
                    body: Some(error_response(id, &e)),
                };
            }
        };
        match self.call(method, &params) {
            Ok(mut result) => {
                if era == Era::Modern {
                    self.decorate(&mut result);
                }
                Reply {
                    http_status: 200,
                    body: Some(json!({ "jsonrpc": "2.0", "id": id, "result": result })),
                }
            }
            Err(e) => Reply {
                http_status: http_status(&era, e.code),
                body: Some(error_response(id, &e)),
            },
        }
    }

    fn decorate(&self, result: &mut Value) {
        let Some(obj) = result.as_object_mut() else {
            return;
        };
        obj.entry("resultType")
            .or_insert_with(|| Value::String("complete".into()));
        let meta = obj
            .entry("_meta")
            .or_insert_with(|| Value::Object(Map::new()));
        if let Some(meta) = meta.as_object_mut() {
            meta.entry(META_SERVER_INFO)
                .or_insert_with(|| server_info(&self.backend.info()));
        }
    }

    fn call(&self, method: &str, params: &Value) -> Result<Value, RpcError> {
        match method {
            "initialize" => Ok(initialize_result(
                &self.backend.info(),
                params.get("protocolVersion").and_then(Value::as_str),
            )),
            "server/discover" => Ok(discover_result(&self.backend.info())),
            "ping" => Ok(json!({})),
            "tools/list" => {
                Ok(json!({ "tools": tool_definitions(self.backend.policy().allow_control) }))
            }
            "tools/call" => self.call_tool(params),
            _ => Err(RpcError::new(
                METHOD_NOT_FOUND,
                format!("method not found: {method}"),
            )),
        }
    }

    fn call_tool(&self, params: &Value) -> Result<Value, RpcError> {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| RpcError::new(INVALID_PARAMS, "missing tool name"))?;
        let args = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        if !args.is_object() {
            return Err(RpcError::new(INVALID_PARAMS, "arguments must be an object"));
        }
        let allow_control = self.backend.policy().allow_control;
        let outcome = match name {
            "get_current_readings" => Ok(self.current_readings()),
            "get_history" => self.history(&args),
            "get_alerts" => self.alerts(),
            "get_device_status" => Ok(self.device_status()),
            "set_alert_threshold" | "identify_device" | "set_relay" | "pulse_relay"
                if !allow_control =>
            {
                Err(
                    "Changes are disabled. Turn on \"Allow changes\" in TOWER Bridge → Settings → API & MCP."
                        .to_string(),
                )
            }
            "set_relay" => self.set_relay(&args),
            "pulse_relay" => self.pulse_relay(&args),
            "set_alert_threshold" => self.set_alert_threshold(&args),
            "identify_device" => self
                .backend
                .identify()
                .map(|()| json!({ "ok": true, "message": "The LED on the TOWER Bridge Module is blinking." }))
                .map_err(api_err),
            _ => {
                return Err(RpcError::new(
                    INVALID_PARAMS,
                    format!("unknown tool: {name}"),
                ));
            }
        };
        Ok(match outcome {
            Ok(data) => json!({
                "content": [{ "type": "text", "text": data.to_string() }],
                "structuredContent": data,
                "isError": false,
            }),
            Err(message) => json!({
                "content": [{ "type": "text", "text": message }],
                "isError": true,
            }),
        })
    }

    fn current_readings(&self) -> Value {
        let device = self.backend.device();
        let (status, message) = device_status(&device);
        let mut out = Map::new();
        out.insert("device_status".into(), json!(status));
        if let Some(m) = message {
            out.insert("message".into(), json!(m));
        }
        if let DeviceState::Ready { revision, .. } = &device {
            out.insert("climate_module".into(), json!(revision.as_str()));
        }
        match self.backend.latest() {
            Some(r) => {
                out.insert("timestamp".into(), json!(format_time(r.ts)));
                out.insert("age_seconds".into(), json!((now_ms() - r.ts).max(0) / 1000));
                out.insert("stale".into(), json!(!device.is_ready()));
                for m in Metric::ALL {
                    if let Some(v) = r.values.get(m) {
                        out.insert(friendly_key(m).into(), json!(friendly(m, v)));
                    }
                }
                if let Some(a) = r.altitude {
                    out.insert("altitude_m".into(), json!(round(a, 1)));
                }
            }
            None => {
                out.insert("message".into(), json!("No measurement available yet."));
            }
        }
        let active: Vec<Value> = self
            .backend
            .active_alerts()
            .iter()
            .map(|a| {
                json!({
                    "metric": a.metric.as_str(),
                    "limit": if a.bound == Bound::High { "above" } else { "below" },
                    "threshold": friendly(a.metric, a.threshold),
                    "value": friendly(a.metric, a.value),
                    "unit": friendly_unit(a.metric),
                    "since": format_time(a.since),
                })
            })
            .collect();
        out.insert("active_alerts".into(), json!(active));
        Value::Object(out)
    }

    fn history(&self, args: &Value) -> Result<Value, String> {
        let now = now_ms();
        let (from, to) = match args.get("from").and_then(Value::as_str) {
            Some(f) => {
                let from = parse_time(f).ok_or_else(|| format!("invalid 'from' time: {f}"))?;
                let to = match args.get("to").and_then(Value::as_str) {
                    Some(t) => parse_time(t).ok_or_else(|| format!("invalid 'to' time: {t}"))?,
                    None => now,
                };
                (from, to)
            }
            None => {
                let hours = args
                    .get("hours")
                    .and_then(Value::as_f64)
                    .unwrap_or(24.0)
                    .clamp(0.05, 9600.0);
                (now - (hours * 3_600_000.0) as i64, now)
            }
        };
        if from >= to {
            return Err("'from' must be before 'to'".into());
        }
        let max_points = args
            .get("max_points")
            .and_then(Value::as_u64)
            .unwrap_or(48)
            .clamp(2, 500) as usize;
        let metrics: Vec<Metric> = match args.get("metrics").and_then(Value::as_array) {
            Some(list) if !list.is_empty() => list
                .iter()
                .map(|v| {
                    v.as_str()
                        .and_then(metric_from_str)
                        .ok_or_else(|| format!("unknown metric: {v}"))
                })
                .collect::<Result<_, _>>()?,
            _ => Metric::ALL.to_vec(),
        };

        let h = self
            .backend
            .history(from, to, max_points)
            .map_err(api_err)?;
        let series = |m: Metric| match m {
            Metric::Temperature => &h.temperature,
            Metric::Humidity => &h.humidity,
            Metric::Illuminance => &h.illuminance,
            Metric::Pressure => &h.pressure,
        };

        let mut stats = Map::new();
        for &m in &metrics {
            let s = series(m);
            let avgs: Vec<f64> = s.avg.iter().flatten().copied().collect();
            if avgs.is_empty() {
                continue;
            }
            let min = s
                .min
                .iter()
                .flatten()
                .copied()
                .fold(f64::INFINITY, f64::min);
            let max = s
                .max
                .iter()
                .flatten()
                .copied()
                .fold(f64::NEG_INFINITY, f64::max);
            let avg = avgs.iter().sum::<f64>() / avgs.len() as f64;
            stats.insert(
                friendly_key(m).into(),
                json!({ "min": friendly(m, min), "avg": friendly(m, avg), "max": friendly(m, max) }),
            );
        }

        let mut points = Vec::new();
        for (i, t) in h.t.iter().enumerate() {
            let mut p = Map::new();
            for &m in &metrics {
                if let Some(Some(v)) = series(m).avg.get(i) {
                    p.insert(friendly_key(m).into(), json!(friendly(m, *v)));
                }
            }
            if p.is_empty() {
                continue;
            }
            // `t` is the bucket centre; report the bucket start.
            let start = (*t * 1000.0) as i64 - h.bucket_ms / 2;
            p.insert("time".into(), json!(format_time(start)));
            points.push(Value::Object(p));
        }

        Ok(json!({
            "from": format_time(from),
            "to": format_time(to),
            "bucket_minutes": round(h.bucket_ms as f64 / 60_000.0, 2),
            "stats": stats,
            "points": points,
            "note": if points.is_empty() { "No readings in this range." } else { "Each point is the average over bucket_minutes starting at 'time'." },
        }))
    }

    fn friendly_rule(m: Metric, r: &AlertRule) -> Value {
        json!({
            "enabled": r.enabled,
            "below": r.min.map(|v| friendly(m, v)),
            "above": r.max.map(|v| friendly(m, v)),
            "hysteresis": friendly(m, r.hysteresis),
            "unit": friendly_unit(m),
            "repeat_after_minutes": round(r.cooldown_s as f64 / 60.0, 1),
            "notify_recovery": r.notify_recovery,
        })
    }

    fn alerts(&self) -> Result<Value, String> {
        let rules = self.backend.alert_rules();
        let mut limits = Map::new();
        for m in Metric::ALL {
            limits.insert(m.as_str().into(), Self::friendly_rule(m, rules.get(m)));
        }
        let active: Vec<Value> = self
            .backend
            .active_alerts()
            .iter()
            .map(|a| {
                json!({
                    "metric": a.metric.as_str(),
                    "limit": if a.bound == Bound::High { "above" } else { "below" },
                    "threshold": friendly(a.metric, a.threshold),
                    "value": friendly(a.metric, a.value),
                    "since": format_time(a.since),
                })
            })
            .collect();
        let recent: Vec<Value> = self
            .backend
            .alert_log(20)
            .map_err(api_err)?
            .iter()
            .map(|e| {
                let m = metric_from_str(&e.metric).unwrap_or(Metric::Temperature);
                json!({
                    "time": format_time(e.ts),
                    "metric": e.metric,
                    "limit": if e.bound == "high" { "above" } else { "below" },
                    "event": if e.kind == "recovered" { "back_to_normal" } else { "triggered" },
                    "value": friendly(m, e.value),
                    "threshold": friendly(m, e.threshold),
                })
            })
            .collect();
        Ok(json!({ "limits": limits, "active": active, "recent": recent }))
    }

    fn device_status(&self) -> Value {
        let device = self.backend.device();
        let (status, message) = device_status(&device);
        let info = self.backend.info();
        let mut out = json!({
            "status": status,
            "sample_interval_s": self.backend.sample_interval_s(),
            "app": { "name": info.name, "version": info.version },
        });
        if let Some(m) = message {
            out["message"] = json!(m);
        }
        if let Some(d) = device.device() {
            out["serial"] = json!(d.serial);
            out["connection"] = json!(match d.backend {
                crate::model::Backend::Hid => "usb-hid",
                crate::model::Backend::I2cDev => "linux-i2c-dev",
            });
        }
        if let DeviceState::Ready {
            revision,
            channel,
            sources,
            ..
        } = &device
        {
            out["climate_module"] = json!(revision.as_str());
            out["i2c_channel"] = json!(channel);
            let sensors: Vec<Value> = sources
                .iter()
                .map(|s| {
                    json!({
                        "metric": s.metric.as_str(),
                        "sensor": s.sensor,
                        "i2c_address": format!("0x{:02X}", s.address),
                    })
                })
                .collect();
            out["sensors"] = json!(sensors);
        }
        out["relay"] = self.backend.relay().map_or(Value::Null, |r| relay_json(&r));
        out
    }

    fn set_relay(&self, args: &Value) -> Result<Value, String> {
        let state = args
            .get("state")
            .and_then(Value::as_bool)
            .ok_or("'state' must be true or false")?;
        let r = self.backend.set_relay(state).map_err(api_err)?;
        Ok(relay_json(&r))
    }

    fn pulse_relay(&self, args: &Value) -> Result<Value, String> {
        let state = match args.get("state") {
            None | Some(Value::Null) => None,
            Some(v) => Some(v.as_bool().ok_or("'state' must be true or false")?),
        };
        let duration = match args.get("duration_ms") {
            None | Some(Value::Null) => None,
            Some(v) => Some(
                v.as_f64()
                    .and_then(pulse_from_ms)
                    .ok_or("'duration_ms' must be a positive number of milliseconds")?,
            ),
        };
        let r = self.backend.pulse_relay(state, duration).map_err(api_err)?;
        Ok(relay_json(&r))
    }

    fn set_alert_threshold(&self, args: &Value) -> Result<Value, String> {
        let metric = args
            .get("metric")
            .and_then(Value::as_str)
            .and_then(metric_from_str)
            .ok_or("'metric' must be one of temperature, humidity, illuminance, pressure")?;
        let mut rule = self.backend.alert_rules().get(metric).clone();
        let number = |key: &str| -> Result<Option<Option<f64>>, String> {
            match args.get(key) {
                None => Ok(None),
                Some(Value::Null) => Ok(Some(None)),
                Some(v) => v
                    .as_f64()
                    .filter(|x| x.is_finite())
                    .map(|x| Some(Some(x)))
                    .ok_or_else(|| format!("'{key}' must be a number or null")),
            }
        };
        if let Some(v) = number("below")? {
            rule.min = v.map(|x| from_friendly(metric, x));
        }
        if let Some(v) = number("above")? {
            rule.max = v.map(|x| from_friendly(metric, x));
        }
        if let Some(v) = number("hysteresis")? {
            let v = v.ok_or("'hysteresis' cannot be null")?;
            if v < 0.0 {
                return Err("'hysteresis' must not be negative".into());
            }
            rule.hysteresis = from_friendly(metric, v);
        }
        if let Some(v) = number("repeat_after_minutes")? {
            let v = v.ok_or("'repeat_after_minutes' cannot be null")?;
            rule.cooldown_s = (v.max(1.0) * 60.0).round() as u64;
        }
        if let Some(v) = args.get("enabled") {
            rule.enabled = v.as_bool().ok_or("'enabled' must be a boolean")?;
        }
        if let Some(v) = args.get("notify_recovery") {
            rule.notify_recovery = v.as_bool().ok_or("'notify_recovery' must be a boolean")?;
        }
        if let (Some(lo), Some(hi)) = (rule.min, rule.max)
            && lo >= hi
        {
            return Err("'below' must be lower than 'above'".into());
        }
        let rules = self.backend.set_alert_rule(metric, rule).map_err(api_err)?;
        Ok(json!({
            "metric": metric.as_str(),
            "limits": Self::friendly_rule(metric, rules.get(metric)),
        }))
    }
}

/// Decode an `Mcp-Name` header value, which may use the Base64 sentinel
/// form `=?base64?<data>?=`.
pub fn decode_header_value(v: &str) -> Option<String> {
    match v
        .strip_prefix("=?base64?")
        .and_then(|s| s.strip_suffix("?="))
    {
        Some(b64) => base64_decode(b64).and_then(|b| String::from_utf8(b).ok()),
        None => Some(v.to_string()),
    }
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => u32::from(c - b'A'),
            b'a'..=b'z' => u32::from(c - b'a') + 26,
            b'0'..=b'9' => u32::from(c - b'0') + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        })
    }
    let bytes: Vec<u8> = s.bytes().filter(|c| *c != b'=').collect();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    for chunk in bytes.chunks(4) {
        let mut acc = 0u32;
        for (i, c) in chunk.iter().enumerate() {
            acc |= val(*c)? << (18 - 6 * i);
        }
        let n = chunk.len();
        if n < 2 {
            return None;
        }
        out.push((acc >> 16) as u8);
        if n > 2 {
            out.push((acc >> 8) as u8);
        }
        if n > 3 {
            out.push(acc as u8);
        }
    }
    Some(out)
}

/// Streamable HTTP header rules for one message. `header` looks up a
/// request header (case-insensitive).
pub fn validate_http_headers(
    msg: &Value,
    header: impl Fn(&str) -> Option<String>,
) -> Result<(), RpcError> {
    let Some(method) = msg.get("method").and_then(Value::as_str) else {
        return Ok(());
    };
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    let body_version = params
        .get("_meta")
        .and_then(|m| m.get(META_VERSION))
        .and_then(Value::as_str);
    let header_version = header("mcp-protocol-version");
    let Some(body_version) = body_version else {
        if let Some(v) = &header_version
            && MODERN_VERSIONS.contains(&v.as_str())
            && msg.get("id").is_some()
        {
            return Err(RpcError::new(
                INVALID_PARAMS,
                format!("missing _meta field {META_VERSION}"),
            ));
        }
        return Ok(());
    };
    let mismatch = |what: &str| RpcError::new(HEADER_MISMATCH, format!("Header mismatch: {what}"));
    if header_version.as_deref() != Some(body_version) {
        return Err(mismatch("MCP-Protocol-Version"));
    }
    if header("mcp-method").as_deref() != Some(method) {
        return Err(mismatch("Mcp-Method"));
    }
    let name_field = match method {
        "tools/call" | "prompts/get" => params.get("name"),
        "resources/read" => params.get("uri"),
        _ => None,
    };
    if let Some(expected) = name_field.and_then(Value::as_str) {
        let got = header("mcp-name").and_then(|v| decode_header_value(&v));
        if got.as_deref() != Some(expected) {
            return Err(mismatch("Mcp-Name"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::api::Policy;
    use crate::api::testing::FakeBackend;

    fn server(allow_control: bool) -> (McpServer, Arc<FakeBackend>) {
        let backend = Arc::new(FakeBackend::new(Policy {
            token: String::new(),
            allow_control,
            mcp: true,
        }));
        (McpServer::new(backend.clone()), backend)
    }

    fn modern_meta() -> Value {
        json!({ META_VERSION: "2026-07-28", META_CLIENT_CAPABILITIES: {} })
    }

    fn result(reply: &Reply) -> &Value {
        reply
            .body
            .as_ref()
            .and_then(|b| b.get("result"))
            .unwrap_or(&Value::Null)
    }

    #[test]
    fn legacy_initialize_negotiates_version() {
        let (s, _) = server(false);
        let r = s.handle(&json!({"jsonrpc":"2.0","id":1,"method":"initialize",
            "params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}));
        assert_eq!(r.http_status, 200);
        assert_eq!(result(&r)["protocolVersion"], "2025-06-18");
        assert_eq!(result(&r)["serverInfo"]["name"], "tower-bridge");
        let r = s.handle(&json!({"jsonrpc":"2.0","id":2,"method":"initialize","params":{"protocolVersion":"2099-01-01"}}));
        assert_eq!(result(&r)["protocolVersion"], LEGACY_VERSIONS[0]);
        assert!(result(&r).get("resultType").is_none());
    }

    #[test]
    fn notifications_get_no_reply() {
        let (s, _) = server(false);
        let r = s.handle(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
        assert_eq!(
            r,
            Reply {
                http_status: 202,
                body: None
            }
        );
    }

    #[test]
    fn modern_discover_and_decoration() {
        let (s, _) = server(false);
        let r = s.handle(&json!({"jsonrpc":"2.0","id":"d","method":"server/discover","params":{"_meta": modern_meta()}}));
        assert_eq!(r.http_status, 200);
        let res = result(&r);
        assert_eq!(res["resultType"], "complete");
        assert!(
            res["supportedVersions"]
                .as_array()
                .is_some_and(|v| v.contains(&json!("2026-07-28")))
        );
        assert_eq!(res["_meta"][META_SERVER_INFO]["name"], "tower-bridge");

        let r = s.handle(&json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{"_meta": modern_meta()}}));
        assert_eq!(result(&r)["resultType"], "complete");
        assert_eq!(
            result(&r)["_meta"][META_SERVER_INFO]["title"],
            "TOWER Bridge"
        );
    }

    #[test]
    fn modern_version_errors() {
        let (s, _) = server(false);
        let r = s.handle(&json!({"jsonrpc":"2.0","id":1,"method":"tools/list",
            "params":{"_meta":{META_VERSION:"1900-01-01", META_CLIENT_CAPABILITIES:{}}}}));
        assert_eq!(r.http_status, 400);
        let err = &r
            .body
            .as_ref()
            .map(|b| b["error"].clone())
            .unwrap_or_default();
        assert_eq!(err["code"], UNSUPPORTED_PROTOCOL_VERSION);
        assert_eq!(err["data"]["requested"], "1900-01-01");

        let r = s.handle(&json!({"jsonrpc":"2.0","id":1,"method":"tools/list",
            "params":{"_meta":{META_VERSION:"2026-07-28"}}}));
        assert_eq!(r.http_status, 400);
        assert_eq!(
            r.body.as_ref().map(|b| b["error"]["code"].clone()),
            Some(json!(INVALID_PARAMS))
        );
    }

    #[test]
    fn unknown_method_status_depends_on_era() {
        let (s, _) = server(false);
        let legacy = s.handle(&json!({"jsonrpc":"2.0","id":1,"method":"resources/list"}));
        assert_eq!(legacy.http_status, 200);
        let modern = s.handle(&json!({"jsonrpc":"2.0","id":1,"method":"resources/list","params":{"_meta": modern_meta()}}));
        assert_eq!(modern.http_status, 404);
        assert_eq!(
            modern.body.as_ref().map(|b| b["error"]["code"].clone()),
            Some(json!(METHOD_NOT_FOUND))
        );
    }

    #[test]
    fn tools_list_respects_control_policy() {
        let names = |allow| -> Vec<String> {
            tool_definitions(allow)
                .iter()
                .filter_map(|t| t["name"].as_str().map(String::from))
                .collect()
        };
        assert!(!names(false).contains(&"set_alert_threshold".to_string()));
        assert!(names(true).contains(&"set_alert_threshold".to_string()));
        assert!(names(true).contains(&"identify_device".to_string()));
        assert!(names(true).contains(&"pulse_relay".to_string()));
        assert!(!names(false).contains(&"set_relay".to_string()));
    }

    #[test]
    fn relay_tools() {
        let tool = |s: &McpServer, name: &str, args: Value| {
            let r = s.handle(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
                "params":{"name": name, "arguments": args}}));
            result(&r).clone()
        };
        let (s, backend) = server(false);
        let status = tool(&s, "get_device_status", json!({}));
        assert_eq!(status["structuredContent"]["relay"]["state"], Value::Null);
        assert_eq!(status["structuredContent"]["relay"]["i2c_address"], "0x3B");
        assert_eq!(
            tool(&s, "set_relay", json!({"state": true}))["isError"],
            true
        );

        let (s, backend2) = server(true);
        let r = tool(&s, "set_relay", json!({"state": true}));
        assert_eq!(r["isError"], false);
        assert_eq!(r["structuredContent"]["state"], true);
        assert_eq!(
            tool(&s, "set_relay", json!({"state": "on"}))["isError"],
            true
        );

        let r = tool(&s, "pulse_relay", json!({"duration_ms": 1500}));
        assert_eq!(r["isError"], false);
        assert!(r["structuredContent"]["pulse_ends"].is_string());
        assert_eq!(r["structuredContent"]["returns_to"], true);
        assert_eq!(
            backend2.pulsed.lock().ok().and_then(|p| *p),
            Some((None, Some(Duration::from_millis(1500))))
        );
        drop(backend);
    }

    #[test]
    fn current_readings_use_friendly_units() {
        let (s, _) = server(false);
        let r = s.handle(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
            "params":{"name":"get_current_readings","arguments":{}}}));
        let res = result(&r);
        assert_eq!(res["isError"], false);
        let data = &res["structuredContent"];
        assert_eq!(data["temperature_c"], 23.46);
        assert_eq!(data["pressure_hpa"], 1003.22);
        assert_eq!(data["climate_module"], "R2");
        assert_eq!(data["active_alerts"][0]["limit"], "above");
        assert!(
            res["content"][0]["text"]
                .as_str()
                .is_some_and(|t| t.contains("temperature_c"))
        );
    }

    #[test]
    fn history_tool() {
        let (s, _) = server(false);
        let r = s.handle(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
            "params":{"name":"get_history","arguments":{"hours":2,"metrics":["temperature","pressure"]}}}));
        let data = &result(&r)["structuredContent"];
        assert_eq!(data["stats"]["temperature_c"]["min"], 21.5);
        assert_eq!(data["stats"]["temperature_c"]["max"], 23.5);
        let p_avg = data["stats"]["pressure_hpa"]["avg"].as_f64().unwrap_or(0.0);
        assert!((p_avg - 1000.0).abs() < 0.02, "{p_avg}");
        assert!(data["stats"].get("humidity_pct").is_none());
        assert_eq!(
            data["points"].as_array().map(Vec::len),
            Some(2),
            "empty bucket skipped"
        );

        let r = s.handle(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
            "params":{"name":"get_history","arguments":{"metrics":["noise"]}}}));
        assert_eq!(result(&r)["isError"], true);
    }

    #[test]
    fn control_tools_are_gated() {
        let (s, backend) = server(false);
        let r = s.handle(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
            "params":{"name":"identify_device","arguments":{}}}));
        assert_eq!(result(&r)["isError"], true);
        assert_eq!(backend.identified.lock().map(|n| *n).unwrap_or(99), 0);

        let (s, backend) = server(true);
        let r = s.handle(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
            "params":{"name":"identify_device","arguments":{}}}));
        assert_eq!(result(&r)["isError"], false);
        assert_eq!(backend.identified.lock().map(|n| *n).unwrap_or(0), 1);
    }

    #[test]
    fn set_alert_threshold_converts_units() {
        let (s, backend) = server(true);
        let r = s.handle(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
            "params":{"name":"set_alert_threshold","arguments":{"metric":"pressure","enabled":true,"below":980,"above":null,"hysteresis":2}}}));
        assert_eq!(result(&r)["isError"], false, "{r:?}");
        let rules = backend.alert_rules();
        assert!(rules.pressure.enabled);
        assert_eq!(rules.pressure.min, Some(98_000.0));
        assert_eq!(rules.pressure.max, None);
        assert_eq!(rules.pressure.hysteresis, 200.0);
        assert_eq!(result(&r)["structuredContent"]["limits"]["below"], 980.0);

        let r = s.handle(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
            "params":{"name":"set_alert_threshold","arguments":{"metric":"temperature","below":30,"above":20}}}));
        assert_eq!(result(&r)["isError"], true);
    }

    #[test]
    fn unknown_tool_is_protocol_error() {
        let (s, _) = server(false);
        let r = s.handle(
            &json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"nope"}}),
        );
        assert_eq!(
            r.body.as_ref().map(|b| b["error"]["code"].clone()),
            Some(json!(INVALID_PARAMS))
        );
    }

    #[test]
    fn header_validation() {
        let msg = json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
            "params":{"name":"get_alerts","_meta": modern_meta()}});
        let ok = |k: &str| match k {
            "mcp-protocol-version" => Some("2026-07-28".to_string()),
            "mcp-method" => Some("tools/call".to_string()),
            "mcp-name" => Some("=?base64?Z2V0X2FsZXJ0cw==?=".to_string()),
            _ => None,
        };
        assert_eq!(validate_http_headers(&msg, ok), Ok(()));
        let bad_name = |k: &str| match k {
            "mcp-name" => Some("get_history".to_string()),
            other => ok(other),
        };
        assert_eq!(
            validate_http_headers(&msg, bad_name).map_err(|e| e.code),
            Err(HEADER_MISMATCH)
        );
        let legacy = json!({"jsonrpc":"2.0","id":1,"method":"tools/list"});
        assert_eq!(
            validate_http_headers(&legacy, |_| Some("2025-06-18".into())),
            Ok(())
        );
        assert_eq!(
            validate_http_headers(&legacy, |k| (k == "mcp-protocol-version")
                .then(|| "2026-07-28".into()))
            .map_err(|e| e.code),
            Err(INVALID_PARAMS)
        );
    }

    #[test]
    fn base64_sentinel() {
        assert_eq!(
            decode_header_value("=?base64?aMOpbGxv?=").as_deref(),
            Some("héllo")
        );
        assert_eq!(decode_header_value("plain").as_deref(), Some("plain"));
    }
}
