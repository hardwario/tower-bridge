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
        "schema": { "type": "string", "enum": ["temperature", "humidity", "illuminance", "pressure", "co2", "voc"] }
    });
    let control = "Requires \"Allow changes\" in the app settings.";
    let error = json!({ "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Error" } } } });
    // Every route: a wrong or missing token (401), a foreign Host or Origin
    // without a token (403).
    let guarded = |mut responses: Value| {
        responses["401"] =
            json!({ "description": "Invalid access token, or one is required (network access)" });
        responses["403"] = json!({ "description": "Not addressed to localhost, a cross-origin request, or changes not allowed" });
        responses
    };
    let unavailable = json!({ "description": "The bridge is not connected or failed", "content": error["content"].clone() });
    json!({
        "openapi": "3.1.0",
        "info": {
            "title": "TOWER Bridge API",
            "version": info.version,
            "description": "Local API of the TOWER Bridge desktop app. Values are SI units: temperature °C, \
    humidity %RH, illuminance lx, pressure Pa, altitude m; CO2 ppm and TVOC ppb. Timestamps are Unix milliseconds. Sensors can sit \
    on I2C channel 0 and 1 of the bridge; readings and alerts carry the channel.",
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
                        "channel": { "type": "integer", "description": "I2C switch channel of the sensors" },
                        "temperature": { "type": ["number", "null"] }, "humidity": { "type": ["number", "null"] },
                        "illuminance": { "type": ["number", "null"] }, "pressure": { "type": ["number", "null"] },
                        "altitude": { "type": ["number", "null"] },
                        "co2": { "type": ["number", "null"], "description": "ppm" },
                        "voc": { "type": ["number", "null"], "description": "ppb TVOC" }
                    }
                },
                "Latest": {
                    "type": "object",
                    "properties": {
                        "ts": { "type": "integer", "description": "Newest reading" }, "time": { "type": "string" },
                        "stale": { "type": "boolean", "description": "The bridge is not working right now" },
                        "readings": { "type": "array", "items": { "$ref": "#/components/schemas/Reading" },
                            "description": "One per I2C channel, in channel order" }
                    }
                },
                "AlertRule": {
                    "type": "object",
                    "properties": {
                        "enabled": { "type": "boolean" },
                        "min": { "type": ["number", "null"] }, "max": { "type": ["number", "null"] },
                        "hysteresis": { "type": "number" }, "cooldown_ms": { "type": "integer" },
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
            "/": { "get": { "summary": "Index of the API", "responses": guarded(json!({ "200": { "description": "Links" } })) } },
            "/api/v1": { "get": { "summary": "Index of the API", "responses": guarded(json!({ "200": { "description": "Links" } })) } },
            "/api/v1/openapi.json": { "get": { "summary": "This document", "responses": guarded(json!({ "200": { "description": "OpenAPI 3.1" } })) } },
            "/api/v1/status": { "get": {
                "summary": "App and device status: version, device state with the module, sensor and address behind each value, \
    the relay, the measurement interval (sample_interval_ms), last reading time, active alerts, whether MCP and changes are on",
                "responses": guarded(json!({ "200": { "description": "Status" } })) } },
            "/api/v1/readings/latest": { "get": {
                "summary": "Latest reading of each I2C channel",
                "responses": guarded(json!({
                    "200": { "description": "Readings", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Latest" } } } },
                    "404": { "description": "No measurement yet (code no_data)", "content": error["content"].clone() }
                }))
            } },
            "/api/v1/readings": { "get": {
                "summary": "Averaged history (columnar; t in seconds; per metric one avg/min/max series for each entry of 'channels')",
                "parameters": range_params.as_array().cloned().unwrap_or_default().into_iter()
                    .chain([json!({ "name": "max_points", "in": "query", "schema": { "type": "integer", "default": 500 },
                        "description": "Approximate number of points; values outside 2 to 5000 are clamped." })])
                    .collect::<Vec<_>>(),
                "responses": guarded(json!({ "200": { "description": "History" },
                    "400": { "description": "Invalid range or hours", "content": error["content"].clone() } }))
            } },
            "/api/v1/readings.csv": { "get": {
                "summary": "Individual readings as CSV, one row per measurement and I2C channel",
                "parameters": range_params,
                "responses": guarded(json!({ "200": { "description": "CSV, streamed", "content": { "text/csv": {} } },
                    "400": { "description": "Invalid range or hours", "content": error["content"].clone() } }))
            } },
            "/api/v1/alerts": { "get": { "summary": "Alert rules and active alerts", "responses": guarded(json!({ "200": { "description": "Alerts" } })) } },
            "/api/v1/alerts/log": { "get": {
                "summary": "Recent alert events",
                "parameters": [{ "name": "limit", "in": "query", "schema": { "type": "integer", "default": 50 },
                    "description": "Values outside 1 to 1000 are clamped." }],
                "responses": guarded(json!({ "200": { "description": "Events, newest first" },
                    "400": { "description": "Invalid limit", "content": error["content"].clone() } }))
            } },
            "/api/v1/alerts/{metric}": {
                "get": { "summary": "Alert rule of one metric", "parameters": [metric_param],
                    "responses": guarded(json!({ "200": { "description": "Rule", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/AlertRule" } } } },
                        "404": { "description": "Unknown metric", "content": error["content"].clone() } })) },
                "patch": { "summary": "Update fields of an alert rule (SI units: pressure in Pa)", "description": control,
                    "parameters": [metric_param],
                    "requestBody": { "content": { "application/json": { "schema": { "$ref": "#/components/schemas/AlertRule" } } } },
                    "responses": guarded(json!({ "200": { "description": "Updated rule" },
                        "400": { "description": "Unknown field, min not below max, or negative hysteresis", "content": error["content"].clone() },
                        "404": { "description": "Unknown metric", "content": error["content"].clone() } })) },
                "put": { "summary": "The same as PATCH", "description": control,
                    "parameters": [metric_param],
                    "requestBody": { "content": { "application/json": { "schema": { "$ref": "#/components/schemas/AlertRule" } } } },
                    "responses": guarded(json!({ "200": { "description": "Updated rule" },
                        "400": { "description": "Unknown field, min not below max, or negative hysteresis", "content": error["content"].clone() },
                        "404": { "description": "Unknown metric", "content": error["content"].clone() } })) }
            },
            "/api/v1/device/identify": { "post": { "summary": "Blink the bridge LED", "description": control,
                "responses": guarded(json!({ "200": { "description": "OK" }, "503": unavailable.clone() })) } },
            "/api/v1/device/rescan": { "post": { "summary": "Look for the sensors again", "description": control,
                "responses": guarded(json!({ "200": { "description": "OK" }, "503": unavailable.clone() })) } },
            "/api/v1/gps": {
                "get": { "summary": "GPS Module: fix, position (WGS 84 degrees), altitude (m above sea level), horizontal accuracy (m), satellites, UTC time (Unix ms)",
                    "responses": guarded(json!({ "200": { "description": "GPS" },
                        "404": { "description": "No GPS Module connected (code no_gps)", "content": error["content"].clone() } })) }
            },
            "/api/v1/relay": {
                "get": { "summary": "Relay Module state (true, false or null when unknown)",
                    "responses": guarded(json!({ "200": { "description": "Relay", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Relay" } } } },
                        "404": { "description": "No Relay Module connected", "content": error["content"].clone() } })) },
                "put": { "summary": "Switch the relay to true or false; ends a running pulse", "description": control,
                    "requestBody": { "required": true, "content": { "application/json": { "schema": {
                        "type": "object", "required": ["state"],
                        "properties": { "state": { "type": "boolean" } } } } } },
                    "responses": guarded(json!({ "200": { "description": "New state" },
                        "400": { "description": "Invalid state", "content": error["content"].clone() },
                        "404": { "description": "No Relay Module connected", "content": error["content"].clone() },
                        "503": unavailable.clone() })) }
            },
            "/api/v1/relay/pulse": { "post": {
                "summary": "Switch the relay to a state for a while, then back",
                "description": format!("{control} Without a state the relay switches to the opposite of its current one \
    (false when unknown), or a running pulse is extended; without duration_ms the length from the app settings is used. \
    A duration outside 500 ms to 1 h is clamped to that range."),
                "requestBody": { "content": { "application/json": { "schema": {
                    "type": "object",
                    "properties": {
                        "state": { "type": "boolean" },
                        "duration_ms": { "type": "integer", "exclusiveMinimum": 0 }
                    } } } } },
                "responses": guarded(json!({ "200": { "description": "State during the pulse (pulse_until set)", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Relay" } } } },
                    "400": { "description": "Invalid state or duration", "content": error["content"].clone() },
                    "404": { "description": "No Relay Module connected", "content": error["content"].clone() },
                    "503": unavailable.clone() })) } },
            "/metrics": { "get": { "summary": "Prometheus metrics", "responses": guarded(json!({ "200": { "description": "Text exposition format" } })) } },
            "/mcp": { "post": { "summary": "MCP endpoint (Streamable HTTP; JSON-RPC batches of up to 32 calls for older protocol versions)",
                "responses": guarded(json!({ "200": { "description": "JSON-RPC response" }, "202": { "description": "Notification accepted" },
                    "400": { "description": "Invalid JSON-RPC, header mismatch or batch too long" } })) } }
        }
    })
}
