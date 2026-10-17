//! OpenAPI 3.1 description of the REST API.

use serde_json::{Value, json};

use super::ApiInfo;

pub fn document(info: &ApiInfo) -> Value {
    let range_params = json!([
        { "name": "from", "in": "query", "schema": { "type": "string" },
          "description": "Start: Unix milliseconds or RFC 3339. Default: 'to' minus 'hours'." },
        { "name": "to", "in": "query", "schema": { "type": "string" },
          "description": "End: Unix milliseconds or RFC 3339. Default: now." },
        { "name": "hours", "in": "query", "schema": { "type": "number", "default": 24 },
          "description": "Range length when 'from' is not given." }
    ]);
    let metric_param = json!({
        "name": "metric", "in": "path", "required": true,
        "schema": { "type": "string", "enum": ["temperature", "humidity", "illuminance", "pressure"] }
    });
    let control = "Requires \"Allow changes\" in the app settings.";
    json!({
        "openapi": "3.1.0",
        "info": {
            "title": "TOWER Bridge API",
            "version": info.version,
            "description": "Local API of the TOWER Bridge desktop app. Values are SI units: temperature °C, \
    humidity %RH, illuminance lx, pressure Pa, altitude m. Timestamps are Unix milliseconds.",
            "license": { "name": "MIT" }
        },
        "components": {
            "securitySchemes": { "bearer": { "type": "http", "scheme": "bearer" } },
            "schemas": {
                "Relay": {
                    "type": "object",
                    "properties": {
                        "state": { "type": ["boolean", "null"],
                            "description": "null = unknown (a latching relay cannot be read back)." },
                        "channel": { "type": "integer", "description": "I2C switch channel" },
                        "address": { "type": "integer", "description": "I2C address of the expander" },
                        "pulse_until": { "type": ["integer", "null"], "description": "End of a running pulse (Unix ms)" }
                    }
                },
                "Reading": {
                    "type": "object",
                    "properties": {
                        "ts": { "type": "integer" }, "time": { "type": "string" }, "device": { "type": "string" },
                        "temperature": { "type": ["number", "null"] }, "humidity": { "type": ["number", "null"] },
                        "illuminance": { "type": ["number", "null"] }, "pressure": { "type": ["number", "null"] },
                        "altitude": { "type": ["number", "null"] }, "stale": { "type": "boolean" }
                    }
                },
                "AlertRule": {
                    "type": "object",
                    "properties": {
                        "enabled": { "type": "boolean" },
                        "min": { "type": ["number", "null"] }, "max": { "type": ["number", "null"] },
                        "hysteresis": { "type": "number" }, "cooldown_s": { "type": "integer" },
                        "notify_recovery": { "type": "boolean" }
                    }
                },
                "Error": {
                    "type": "object",
                    "properties": { "error": { "type": "object",
                        "properties": { "code": { "type": "string" }, "message": { "type": "string" } } } }
                }
            }
        },
        "security": [{}, { "bearer": [] }],
        "paths": {
            "/api/v1/status": { "get": { "summary": "App and device status", "responses": { "200": { "description": "Status" } } } },
            "/api/v1/readings/latest": { "get": {
                "summary": "Latest reading",
                "responses": {
                    "200": { "description": "Reading", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Reading" } } } },
                    "404": { "description": "No measurement yet" }
                }
            } },
            "/api/v1/readings": { "get": {
                "summary": "Averaged history (columnar; t in seconds, avg/min/max per metric)",
                "parameters": range_params.as_array().cloned().unwrap_or_default().into_iter()
                    .chain([json!({ "name": "max_points", "in": "query", "schema": { "type": "integer", "default": 500 } })])
                    .collect::<Vec<_>>(),
                "responses": { "200": { "description": "History" } }
            } },
            "/api/v1/readings.csv": { "get": {
                "summary": "Individual readings as CSV",
                "parameters": range_params,
                "responses": { "200": { "description": "CSV", "content": { "text/csv": {} } } }
            } },
            "/api/v1/alerts": { "get": { "summary": "Alert rules and active alerts", "responses": { "200": { "description": "Alerts" } } } },
            "/api/v1/alerts/log": { "get": {
                "summary": "Recent alert events",
                "parameters": [{ "name": "limit", "in": "query", "schema": { "type": "integer", "default": 50 } }],
                "responses": { "200": { "description": "Events, newest first" } }
            } },
            "/api/v1/alerts/{metric}": {
                "get": { "summary": "Alert rule of one metric", "parameters": [metric_param],
                    "responses": { "200": { "description": "Rule", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/AlertRule" } } } } } },
                "patch": { "summary": "Update fields of an alert rule (SI units)", "description": control,
                    "parameters": [metric_param],
                    "requestBody": { "content": { "application/json": { "schema": { "$ref": "#/components/schemas/AlertRule" } } } },
                    "responses": { "200": { "description": "Updated rule" }, "403": { "description": "Changes disabled" } } }
            },
            "/api/v1/device/identify": { "post": { "summary": "Blink the bridge LED", "description": control,
                "responses": { "200": { "description": "OK" }, "403": { "description": "Changes disabled" } } } },
            "/api/v1/device/rescan": { "post": { "summary": "Re-detect the Climate Module", "description": control,
                "responses": { "200": { "description": "OK" }, "403": { "description": "Changes disabled" } } } },
            "/api/v1/relay": {
                "get": { "summary": "Relay Module state (true, false or null when unknown)",
                    "responses": { "200": { "description": "Relay", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Relay" } } } },
                        "404": { "description": "No Relay Module connected" } } },
                "put": { "summary": "Switch the relay to true or false; ends a running pulse", "description": control,
                    "requestBody": { "required": true, "content": { "application/json": { "schema": {
                        "type": "object", "required": ["state"],
                        "properties": { "state": { "type": "boolean" } } } } } },
                    "responses": { "200": { "description": "New state" }, "400": { "description": "Invalid state" },
                        "403": { "description": "Changes disabled" }, "404": { "description": "No Relay Module connected" } } }
            },
            "/api/v1/relay/pulse": { "post": {
                "summary": "Switch the relay to a state for a while, then back",
                "description": format!("{control} Without a state the relay switches to the opposite of its current one \
    (false when unknown); without duration_ms the length from the app settings is used (500 ms to 1 h)."),
                "requestBody": { "content": { "application/json": { "schema": {
                    "type": "object",
                    "properties": {
                        "state": { "type": "boolean" },
                        "duration_ms": { "type": "integer", "minimum": 500, "maximum": 3_600_000 }
                    } } } } },
                "responses": { "200": { "description": "State during the pulse (pulseUntil set)" },
                    "403": { "description": "Changes disabled" }, "404": { "description": "No Relay Module connected" } } } },
            "/metrics": { "get": { "summary": "Prometheus metrics", "responses": { "200": { "description": "Text exposition format" } } } },
            "/mcp": { "post": { "summary": "MCP endpoint (Streamable HTTP)", "responses": { "200": { "description": "JSON-RPC response" } } } }
        }
    })
}
