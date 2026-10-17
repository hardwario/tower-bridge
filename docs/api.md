# Local API and MCP server

TOWER Bridge can run a small HTTP server with:

- a **REST API** (`/api/v1/…`) for scripts, dashboards and home automation,
- **Prometheus metrics** (`/metrics`) for Grafana and friends,
- an **MCP server** (`/mcp`) so AI assistants can read the sensors.

Everything is off by default: the API server, the MCP endpoint (a separate
switch) and the MQTT broker. Turn them on in **Settings → API & MCP**. The
server can be turned off or moved to another port at any time; no restart is
needed.

## Access

| Setting | Effect |
|---|---|
| *Allow network access* off (default) | Listens on `127.0.0.1` only. Local clients need no token. |
| *Allow network access* on | Listens on all interfaces. Every request must carry the token. |
| *Allow changes* off (default) | Read-only. |
| *Allow changes* on | Clients may change alert limits, blink the bridge LED and re-detect the module. |

Send the token from the settings page as `Authorization: Bearer <token>`. A
request with a valid token is always accepted.

Requests without a token are accepted only if they come over loopback and their
`Host` header (and `Origin`, if present) names `localhost`, `127.0.0.1` or
`[::1]`. That blocks web pages from reaching the API through DNS rebinding or
cross-site requests.

## REST API

Values are SI units: temperature °C, relative humidity %, illuminance lx,
pressure **Pa**, altitude m. Timestamps and durations are milliseconds.
JSON keys are lowercase with underscores (`bucket_ms`, `pulse_until`, …). Time parameters
accept Unix milliseconds or RFC 3339. The OpenAPI description is at
`/api/v1/openapi.json`.

| Method | Path | Description |
|---|---|---|
| GET | `/api/v1/status` | App version, device state (with the sensor and I²C address behind each value), last reading time, active alerts |
| GET | `/api/v1/readings/latest` | Latest reading (`stale: true` while the bridge is disconnected) |
| GET | `/api/v1/readings?hours=24&max_points=500` | Averaged history (`from`, `to` also accepted), columnar like the dashboard charts |
| GET | `/api/v1/readings.csv?from=…&to=…` | Individual readings as CSV |
| GET | `/api/v1/alerts` | Alert rules and active alerts |
| GET | `/api/v1/alerts/log?limit=50` | Recent alert events |
| GET | `/api/v1/alerts/{metric}` | One rule (`temperature`, `humidity`, `illuminance`, `pressure`) |
| PATCH / PUT | `/api/v1/alerts/{metric}` | Update fields of a rule, e.g. `{"enabled": true, "max": 26}` *(changes)* |
| POST | `/api/v1/device/identify` | Blink the bridge LED *(changes)* |
| POST | `/api/v1/device/rescan` | Re-detect the Climate Module *(changes)* |
| GET | `/api/v1/relay` | Relay Module state: `{"state": true, "channel": 0, "address": 59, "pulse_until": null}`; `state` is `true`, `false` or `null` (unknown); 404 without a relay |
| PUT | `/api/v1/relay` | `{"state": true}` or `false`; ends a running pulse *(changes)* |
| POST | `/api/v1/relay/pulse` | Switch to a state for a while, then back. Body optional: `{"state": false, "duration_ms": 2000}`; defaults are the opposite state and the configured length *(changes)* |
| GET | `/metrics` | Prometheus text format |

Errors are JSON: `{"error": {"code": "…", "message": "…"}}`.

```sh
curl http://127.0.0.1:8180/api/v1/readings/latest
curl "http://127.0.0.1:8180/api/v1/readings.csv?from=2026-10-01T00:00:00Z&to=2026-10-02T00:00:00Z" -o day.csv
curl -X PATCH http://127.0.0.1:8180/api/v1/alerts/humidity \
     -H "Content-Type: application/json" -d '{"enabled": true, "min": 35, "max": 60}'
curl -X PUT http://127.0.0.1:8180/api/v1/relay \
     -H "Content-Type: application/json" -d '{"state": false}'
curl -X POST http://127.0.0.1:8180/api/v1/relay/pulse
```

The relay state is `null` until the relay has been switched: it is latching
and cannot be read back. `/metrics` exports a known state as
`tower_bridge_relay_state 1` (`true`) or `0` (`false`).

Prometheus scrape config:

```yaml
scrape_configs:
  - job_name: tower-bridge
    static_configs:
      - targets: ["127.0.0.1:8180"]
```

## MCP server

The tools use friendly units (°C, %, lx, **hPa**):

| Tool | Description |
|---|---|
| `get_current_readings` | Latest values, measurement time, active alerts |
| `get_history` | Averaged points and min/avg/max for `hours` back or a `from`/`to` range |
| `get_alerts` | Alert limits, active alerts, recent events |
| `get_device_status` | Bridge and Climate Module status, serial number, sensors and their I²C addresses, interval, relay state |
| `set_alert_threshold` | Change one metric's limits *(only with "Allow changes")* |
| `identify_device` | Blink the bridge LED *(only with "Allow changes")* |
| `set_relay` | Switch the Relay Module: `state` `true` or `false` *(only with "Allow changes")* |
| `pulse_relay` | Pulse the relay, optional `state` and `duration_ms` *(only with "Allow changes")* |

Supported protocol versions:
- `2026-07-28`: stateless; per-request `_meta`, `server/discover`, Streamable HTTP header validation.
- `2025-11-25`, `2025-06-18`, `2025-03-26`, `2024-11-05`: the older `initialize` handshake.

Turn on both *Enable API server* and *MCP server* in the settings.

### Claude Code, Cursor, VS Code (HTTP)

```sh
claude mcp add --transport http tower-bridge http://127.0.0.1:8180/mcp
```

Other clients: add an HTTP / Streamable HTTP MCP server with the URL
`http://127.0.0.1:8180/mcp`. With network access enabled, add the header
`Authorization: Bearer <token>`.

### Claude Desktop and other stdio clients

These clients start the server as a local process. The TOWER Bridge executable
acts as a bridge when started with `--mcp-stdio`. It needs the app running with
the API server enabled, and reads the port and token from the app's settings.
The settings page shows the exact snippet. On macOS:

```json
{
  "mcpServers": {
    "tower-bridge": {
      "command": "/Applications/TOWER Bridge.app/Contents/MacOS/tower-bridge",
      "args": ["--mcp-stdio"]
    }
  }
}
```

On Windows, use the path of the portable exe. On Linux, use the path of the
AppImage. If the app is not running, the tools say so instead of failing
silently.

### Testing

```sh
npx @modelcontextprotocol/inspector --cli http://127.0.0.1:8180/mcp --transport http --method tools/list
```
