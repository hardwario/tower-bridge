//! REST endpoints, Prometheus metrics and the MCP HTTP endpoint.

use std::fmt::Write as _;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

use super::mcp::{self, McpServer, RpcError};
use super::security::{self, Access};
use super::{ApiBackend, ApiError, format_time, metric_from_str, parse_time};
use crate::alerts::{AlertRule, Bound};
use crate::model::Metric;
use crate::relay::{RelayInfo, pulse_from_ms};

#[derive(Clone)]
pub(crate) struct Ctx {
    pub backend: Arc<dyn ApiBackend>,
    pub mcp: Arc<McpServer>,
    pub lan: bool,
}

pub(crate) struct HttpError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl HttpError {
    fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }

    fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "bad_request", message)
    }
}

impl From<ApiError> for HttpError {
    fn from(e: ApiError) -> Self {
        let (status, code) = match &e {
            ApiError::NotFound(_) => (StatusCode::NOT_FOUND, "not_found"),
            ApiError::BadRequest(_) => (StatusCode::BAD_REQUEST, "bad_request"),
            ApiError::Forbidden(_) => (StatusCode::FORBIDDEN, "forbidden"),
            ApiError::Unavailable(_) => (StatusCode::SERVICE_UNAVAILABLE, "unavailable"),
            ApiError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal"),
        };
        Self::new(status, code, e.to_string())
    }
}

impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        let mut r = (
            self.status,
            Json(json!({ "error": { "code": self.code, "message": self.message } })),
        )
            .into_response();
        if self.status == StatusCode::UNAUTHORIZED {
            r.headers_mut()
                .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        }
        r
    }
}

type HttpResult<T> = Result<T, HttpError>;

async fn blocking<T, F>(f: F) -> HttpResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, ApiError> + Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| HttpError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?
        .map_err(Into::into)
}

pub(crate) fn router(ctx: Ctx) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/api/v1", get(index))
        .route("/api/v1/status", get(status))
        .route("/api/v1/readings/latest", get(latest))
        .route("/api/v1/readings", get(readings))
        .route("/api/v1/readings.csv", get(readings_csv))
        .route("/api/v1/alerts", get(alerts))
        .route("/api/v1/alerts/log", get(alert_log))
        .route(
            "/api/v1/alerts/{metric}",
            get(alert_rule)
                .put(update_alert_rule)
                .patch(update_alert_rule),
        )
        .route("/api/v1/device/identify", post(identify))
        .route("/api/v1/device/rescan", post(rescan))
        .route("/api/v1/relay", get(relay).put(set_relay))
        .route("/api/v1/relay/pulse", post(pulse_relay))
        .route("/api/v1/openapi.json", get(openapi))
        .route("/metrics", get(metrics))
        .route(
            "/mcp",
            post(mcp_post)
                .get(mcp_method_not_allowed)
                .delete(mcp_method_not_allowed),
        )
        .fallback(not_found)
        .layer(middleware::from_fn_with_state(ctx.clone(), guard))
        .with_state(ctx)
}

async fn guard(State(ctx): State<Ctx>, req: Request, next: Next) -> Response {
    match security::check(req.headers(), &ctx.backend.policy(), ctx.lan) {
        Access::Token | Access::Local => next.run(req).await,
        Access::Denied(status, message) => {
            HttpError::new(status, "unauthorized", message).into_response()
        }
    }
}

fn require_control(ctx: &Ctx) -> HttpResult<()> {
    if ctx.backend.policy().allow_control {
        Ok(())
    } else {
        Err(HttpError::new(
            StatusCode::FORBIDDEN,
            "control_disabled",
            "Changes are disabled. Enable \"Allow changes\" in TOWER Bridge settings.",
        ))
    }
}

async fn not_found() -> HttpError {
    HttpError::new(StatusCode::NOT_FOUND, "not_found", "no such endpoint")
}

async fn index(State(ctx): State<Ctx>) -> Json<Value> {
    let info = ctx.backend.info();
    let policy = ctx.backend.policy();
    Json(json!({
        "name": info.name,
        "version": info.version,
        "endpoints": {
            "status": "/api/v1/status",
            "latest": "/api/v1/readings/latest",
            "history": "/api/v1/readings?hours=24",
            "csv": "/api/v1/readings.csv?hours=24",
            "alerts": "/api/v1/alerts",
            "alert_log": "/api/v1/alerts/log",
            "relay": "/api/v1/relay",
            "metrics": "/metrics",
            "openapi": "/api/v1/openapi.json",
            "mcp": if policy.mcp { Some("/mcp") } else { None },
        },
        "allow_control": policy.allow_control,
    }))
}

async fn status(State(ctx): State<Ctx>) -> Json<Value> {
    let info = ctx.backend.info();
    let policy = ctx.backend.policy();
    Json(json!({
        "name": info.name,
        "version": info.version,
        "device": ctx.backend.device(),
        "relay": ctx.backend.relay(),
        "latest_ts": ctx.backend.latest().map(|r| r.ts),
        "sample_interval_s": ctx.backend.sample_interval_s(),
        "active_alerts": ctx.backend.active_alerts(),
        "mcp": policy.mcp,
        "allow_control": policy.allow_control,
    }))
}

async fn latest(State(ctx): State<Ctx>) -> HttpResult<Json<Value>> {
    let r = ctx
        .backend
        .latest()
        .ok_or_else(|| HttpError::new(StatusCode::NOT_FOUND, "no_data", "no measurement yet"))?;
    let mut v = serde_json::to_value(&r).map_err(|e| {
        HttpError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string())
    })?;
    v["time"] = json!(format_time(r.ts));
    v["stale"] = json!(!ctx.backend.device().is_ready());
    Ok(Json(v))
}

#[derive(Debug, Default, Deserialize)]
struct RangeQuery {
    from: Option<String>,
    to: Option<String>,
    hours: Option<f64>,
    #[serde(alias = "maxPoints")]
    max_points: Option<usize>,
}

fn range(q: &RangeQuery) -> HttpResult<(i64, i64)> {
    let now = crate::acquisition::unix_ms();
    let to = match &q.to {
        Some(t) => {
            parse_time(t).ok_or_else(|| HttpError::bad_request(format!("invalid 'to': {t}")))?
        }
        None => now,
    };
    let from = match (&q.from, q.hours) {
        (Some(f), _) => {
            parse_time(f).ok_or_else(|| HttpError::bad_request(format!("invalid 'from': {f}")))?
        }
        (None, Some(h)) if h.is_finite() && h > 0.0 => to - (h * 3_600_000.0) as i64,
        (None, Some(_)) => return Err(HttpError::bad_request("'hours' must be positive")),
        (None, None) => to - 86_400_000,
    };
    if from >= to {
        return Err(HttpError::bad_request("'from' must be before 'to'"));
    }
    Ok((from, to))
}

async fn readings(State(ctx): State<Ctx>, Query(q): Query<RangeQuery>) -> HttpResult<Json<Value>> {
    let (from, to) = range(&q)?;
    let max_points = q.max_points.unwrap_or(500).clamp(2, 5000);
    let backend = ctx.backend.clone();
    let h = blocking(move || backend.history(from, to, max_points)).await?;
    Ok(Json(serde_json::to_value(h).unwrap_or_default()))
}

async fn readings_csv(State(ctx): State<Ctx>, Query(q): Query<RangeQuery>) -> HttpResult<Response> {
    let (from, to) = range(&q)?;
    let backend = ctx.backend.clone();
    let csv = blocking(move || backend.csv(from, to)).await?;
    Ok((
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"tower-bridge.csv\"",
            ),
        ],
        csv,
    )
        .into_response())
}

async fn alerts(State(ctx): State<Ctx>) -> Json<Value> {
    Json(json!({
        "rules": ctx.backend.alert_rules(),
        "active": ctx.backend.active_alerts(),
    }))
}

#[derive(Deserialize)]
struct LogQuery {
    limit: Option<usize>,
}

async fn alert_log(State(ctx): State<Ctx>, Query(q): Query<LogQuery>) -> HttpResult<Json<Value>> {
    let limit = q.limit.unwrap_or(50).clamp(1, 1000);
    let backend = ctx.backend.clone();
    let log = blocking(move || backend.alert_log(limit)).await?;
    Ok(Json(serde_json::to_value(log).unwrap_or_default()))
}

fn parse_metric(s: &str) -> HttpResult<Metric> {
    metric_from_str(s).ok_or_else(|| {
        HttpError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("unknown metric: {s}"),
        )
    })
}

async fn alert_rule(
    State(ctx): State<Ctx>,
    Path(metric): Path<String>,
) -> HttpResult<Json<AlertRule>> {
    let m = parse_metric(&metric)?;
    Ok(Json(ctx.backend.alert_rules().get(m).clone()))
}

/// Merge the given fields into the current rule (PUT and PATCH alike).
async fn update_alert_rule(
    State(ctx): State<Ctx>,
    Path(metric): Path<String>,
    Json(patch): Json<Value>,
) -> HttpResult<Json<AlertRule>> {
    require_control(&ctx)?;
    let m = parse_metric(&metric)?;
    let Value::Object(fields) = patch else {
        return Err(HttpError::bad_request("expected a JSON object"));
    };
    let current = ctx.backend.alert_rules().get(m).clone();
    let mut merged = serde_json::to_value(current).unwrap_or_default();
    for (k, v) in fields {
        if !matches!(
            k.as_str(),
            "enabled" | "min" | "max" | "hysteresis" | "cooldown_s" | "notify_recovery"
        ) {
            return Err(HttpError::bad_request(format!("unknown field: {k}")));
        }
        merged[k] = v;
    }
    let rule: AlertRule =
        serde_json::from_value(merged).map_err(|e| HttpError::bad_request(e.to_string()))?;
    if rule.hysteresis < 0.0 {
        return Err(HttpError::bad_request("hysteresis must not be negative"));
    }
    if let (Some(lo), Some(hi)) = (rule.min, rule.max)
        && lo >= hi
    {
        return Err(HttpError::bad_request("min must be lower than max"));
    }
    let backend = ctx.backend.clone();
    let rules = blocking(move || backend.set_alert_rule(m, rule)).await?;
    Ok(Json(rules.get(m).clone()))
}

async fn identify(State(ctx): State<Ctx>) -> HttpResult<Json<Value>> {
    require_control(&ctx)?;
    ctx.backend.identify()?;
    Ok(Json(json!({ "ok": true })))
}

async fn rescan(State(ctx): State<Ctx>) -> HttpResult<Json<Value>> {
    require_control(&ctx)?;
    ctx.backend.rescan()?;
    Ok(Json(json!({ "ok": true })))
}

fn no_relay() -> HttpError {
    HttpError::new(
        StatusCode::NOT_FOUND,
        "no_relay",
        "no Relay Module is connected",
    )
}

async fn relay(State(ctx): State<Ctx>) -> HttpResult<Json<RelayInfo>> {
    ctx.backend.relay().map(Json).ok_or_else(no_relay)
}

fn state_field(body: &Value) -> HttpResult<Option<bool>> {
    match body.get("state") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_bool()
            .map(Some)
            .ok_or_else(|| HttpError::bad_request("'state' must be true or false")),
    }
}

/// `{"state": true | false}`
async fn set_relay(State(ctx): State<Ctx>, Json(body): Json<Value>) -> HttpResult<Json<RelayInfo>> {
    require_control(&ctx)?;
    let state = state_field(&body)?
        .ok_or_else(|| HttpError::bad_request("'state' (true or false) is required"))?;
    if ctx.backend.relay().is_none() {
        return Err(no_relay());
    }
    let backend = ctx.backend.clone();
    Ok(Json(blocking(move || backend.set_relay(state)).await?))
}

/// `{"state"?: true | false, "duration_ms"?: number}`; an empty body is fine.
async fn pulse_relay(State(ctx): State<Ctx>, body: Bytes) -> HttpResult<Json<RelayInfo>> {
    require_control(&ctx)?;
    let body: Value = if body.iter().all(u8::is_ascii_whitespace) {
        json!({})
    } else {
        serde_json::from_slice(&body).map_err(|e| HttpError::bad_request(e.to_string()))?
    };
    let state = state_field(&body)?;
    let duration = match body.get("duration_ms") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_f64().and_then(pulse_from_ms).ok_or_else(|| {
            HttpError::bad_request("'duration_ms' must be a positive number of milliseconds")
        })?),
    };
    if ctx.backend.relay().is_none() {
        return Err(no_relay());
    }
    let backend = ctx.backend.clone();
    Ok(Json(
        blocking(move || backend.pulse_relay(state, duration)).await?,
    ))
}

async fn openapi(State(ctx): State<Ctx>) -> Json<Value> {
    Json(super::openapi::document(&ctx.backend.info()))
}

fn escape_label(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

async fn metrics(State(ctx): State<Ctx>) -> Response {
    let mut out = String::new();
    let device = ctx.backend.device();
    let ready = device.is_ready();
    let _ = writeln!(
        out,
        "# HELP tower_bridge_device_ready Whether the bridge and Climate Module are working."
    );
    let _ = writeln!(out, "# TYPE tower_bridge_device_ready gauge");
    let _ = writeln!(out, "tower_bridge_device_ready {}", u8::from(ready));
    if let (true, Some(r)) = (ready, ctx.backend.latest()) {
        let dev = escape_label(&r.device);
        let gauges = [
            ("temperature_celsius", "Temperature", r.values.temperature),
            (
                "relative_humidity_percent",
                "Relative humidity",
                r.values.humidity,
            ),
            ("illuminance_lux", "Illuminance", r.values.illuminance),
            ("pressure_pascals", "Barometric pressure", r.values.pressure),
            (
                "altitude_meters",
                "Altitude estimated from pressure",
                r.altitude,
            ),
        ];
        for (name, help, value) in gauges {
            if let Some(v) = value {
                let _ = writeln!(out, "# HELP tower_bridge_{name} {help}.");
                let _ = writeln!(out, "# TYPE tower_bridge_{name} gauge");
                let _ = writeln!(out, "tower_bridge_{name}{{device=\"{dev}\"}} {v}");
            }
        }
        let _ = writeln!(
            out,
            "# HELP tower_bridge_last_reading_timestamp_seconds Time of the latest reading."
        );
        let _ = writeln!(
            out,
            "# TYPE tower_bridge_last_reading_timestamp_seconds gauge"
        );
        let _ = writeln!(
            out,
            "tower_bridge_last_reading_timestamp_seconds{{device=\"{dev}\"}} {}",
            r.ts as f64 / 1000.0
        );
    }
    // Unknown (never switched) is left out rather than guessed.
    if let Some(state) = ctx.backend.relay().and_then(|r| r.state) {
        let _ = writeln!(
            out,
            "# HELP tower_bridge_relay_state Relay Module state: 1 true, 0 false."
        );
        let _ = writeln!(out, "# TYPE tower_bridge_relay_state gauge");
        let _ = writeln!(out, "tower_bridge_relay_state {}", u8::from(state));
    }
    let _ = writeln!(
        out,
        "# HELP tower_bridge_alert_active Active threshold alerts."
    );
    let _ = writeln!(out, "# TYPE tower_bridge_alert_active gauge");
    let active_alerts = ctx.backend.active_alerts();
    for m in Metric::ALL {
        for (bound, label) in [(Bound::Low, "low"), (Bound::High, "high")] {
            let active = active_alerts
                .iter()
                .any(|a| a.metric == m && a.bound == bound);
            let _ = writeln!(
                out,
                "tower_bridge_alert_active{{metric=\"{}\",bound=\"{label}\"}} {}",
                m.as_str(),
                u8::from(active)
            );
        }
    }
    (
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        out,
    )
        .into_response()
}

async fn mcp_method_not_allowed(State(ctx): State<Ctx>) -> Response {
    if !ctx.backend.policy().mcp {
        return not_found().await.into_response();
    }
    (StatusCode::METHOD_NOT_ALLOWED, [(header::ALLOW, "POST")]).into_response()
}

async fn mcp_post(State(ctx): State<Ctx>, headers: HeaderMap, body: Bytes) -> Response {
    if !ctx.backend.policy().mcp {
        return not_found().await.into_response();
    }
    let msg: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            let err = RpcError::new(mcp::PARSE_ERROR, format!("parse error: {e}"));
            return (
                StatusCode::BAD_REQUEST,
                Json(mcp::error_response(Value::Null, &err)),
            )
                .into_response();
        }
    };
    let server = ctx.mcp.clone();
    if let Value::Array(items) = msg {
        // JSON-RPC batch (legacy protocol versions).
        let replies = tokio::task::spawn_blocking(move || {
            items
                .iter()
                .filter_map(|m| server.handle(m).body)
                .collect::<Vec<_>>()
        })
        .await
        .unwrap_or_default();
        return if replies.is_empty() {
            StatusCode::ACCEPTED.into_response()
        } else {
            Json(Value::Array(replies)).into_response()
        };
    }
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(String::from)
    };
    if let Err(e) = mcp::validate_http_headers(&msg, header) {
        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        return (StatusCode::BAD_REQUEST, Json(mcp::error_response(id, &e))).into_response();
    }
    let reply = match tokio::task::spawn_blocking(move || server.handle(&msg)).await {
        Ok(r) => r,
        Err(e) => {
            let err = RpcError::new(mcp::INTERNAL_ERROR, e.to_string());
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(mcp::error_response(Value::Null, &err)),
            )
                .into_response();
        }
    };
    let status = StatusCode::from_u16(reply.http_status).unwrap_or(StatusCode::OK);
    match reply.body {
        Some(body) => (status, Json(body)).into_response(),
        None => StatusCode::ACCEPTED.into_response(),
    }
}
