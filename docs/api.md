# Local API and MCP server

TOWER Bridge can run a small HTTP server with:

- a **REST API** (`/api/v1/…`) for scripts, dashboards and home automation,
- **Prometheus metrics** (`/metrics`) for Grafana and friends,
- an **MCP server** (`/mcp`) so AI assistants can read the sensors.

Everything is off by default: the API server and the MCP endpoint (a separate
switch). Turn them on in **Settings → API & MCP** (MQTT is in **Settings →
MQTT**), or with `tower-bridge-cli config set api.enabled true` and
`api.mcp true` for `tower-bridge-cli run`. The server can be turned off or
moved to another port at any time; no restart is needed.

## Access

| Setting | Effect |
|---|---|
| *Allow network access* off (default) | Listens on `127.0.0.1` only. Local clients need no token. |
| *Allow network access* on | Listens on all interfaces. Every request must carry the token. |
| *Allow changes* off (default) | Read-only. |
| *Allow changes* on | Clients may change alert limits, blink the bridge LED, look for the sensors again, switch the relay and write the URL on the NFC Tag (which phones then open). |

Send the token from the settings page as `Authorization: Bearer <token>`. A
request with a valid token is always accepted.

Requests without a token are accepted only if they come over loopback and their
`Host` header names `localhost`, `127.0.0.1` or `[::1]`, and their `Origin`, if
present, is an `http` or `https` page on one of those. That blocks web pages
from reaching the API through DNS rebinding or cross-site requests.

With network access on, the traffic is plain HTTP: anyone who can watch the
network sees the token and the readings, and with *Allow changes* on can
switch the relay or rewrite the NFC Tag with that token. Use it on a network
you trust.

The server serves up to 64 connections at once and closes a connection after
30 seconds without traffic.

## REST API

Values are SI units: temperature °C, relative humidity %, illuminance lx,
pressure **Pa**, altitude m; CO₂ in ppm and TVOC in ppb. Timestamps and
durations are milliseconds (`_ms`).
JSON keys are lowercase with underscores (`bucket_ms`, `pulse_until`, …). Time parameters
accept Unix milliseconds or RFC 3339. The OpenAPI description is at
`/api/v1/openapi.json`.

Sensors can sit on I²C channel 0 and 1 of the Bridge Module, for example a
Climate Module on channel 0 and sensor tags on channel 1. Readings, sensors and
alerts carry their `channel`.

| Method | Path | Description |
|---|---|---|
| GET | `/api/v1/status` | App version, device state (with the channel, module, sensor and I²C address behind each value), the relay, the NFC Tag (`nfc`), the MQTT connection (`mqtt`: `{"status": "connected", "address": "broker:1883"}`, or `disabled`, `connecting` with the last `error`, `error` with a `message`), `sample_interval_ms`, last reading time, active alerts, whether `mcp` and `allow_control` are on |
| GET | `/api/v1/readings/latest` | Latest reading of each channel (`stale: true` while the bridge is disconnected); 404 `no_data` before the first measurement |
| GET | `/api/v1/readings?hours=24&max_points=500` | Averaged history (`from`, `to` also accepted), columnar like the dashboard charts |
| GET | `/api/v1/readings.csv?from=…&to=…` | Individual readings as CSV, one row per measurement and channel, as the app exports them: the temperature in the unit of the settings (`temperature_c` or `temperature_f`), pressure in hPa; the headers name the units |
| GET | `/api/v1/alerts` | Alert rules and active alerts |
| GET | `/api/v1/alerts/log?limit=50` | Recent alert events |
| GET | `/api/v1/alerts/{metric}` | One rule (`temperature`, `humidity`, `illuminance`, `pressure`, `co2`, `voc`) |
| PATCH / PUT | `/api/v1/alerts/{metric}` | Update fields of a rule: `enabled`, `min`, `max`, `hysteresis`, `cooldown_ms`, `notify_recovery`, in SI units (pressure in Pa), e.g. `{"enabled": true, "max": 26}` *(changes)* |
| POST | `/api/v1/device/identify` | Blink the bridge LED *(changes)* |
| POST | `/api/v1/device/rescan` | Look for the sensors again *(changes)* |
| GET | `/api/v1/relay` | Relay Module state: `{"state": true, "channel": 0, "address": 59, "pulse_until": null}`; `state` is `true`, `false` or `null` (unknown); 404 without a relay |
| PUT | `/api/v1/relay` | `{"state": true}` or `false`; ends a running pulse *(changes)* |
| POST | `/api/v1/relay/pulse` | Switch to a state for a while, then back. Body optional: `{"state": false, "duration_ms": 2000}`; defaults are the opposite state (or, while a pulse runs, extending it) and the configured length *(changes)* |
| GET | `/api/v1/gps` | GPS Module: `{"channel": 0, "fix": true, "satellites": 9, "latitude": 50.771168, "longitude": 15.056328, "altitude": 374.2, "accuracy": 3.1, "time": 1792245600000}`; position, altitude (m above sea level) and accuracy (m) are `null` without a fix; 404 without a GPS Module |
| GET | `/api/v1/nfc` | TOWER NFC Tag: `{"channel": 0, "address": 8, "part": "NT3H2111", "url": "https://www.hardwario.com", "formatted": true}`; `url` is `null` when the tag holds none; 404 `no_nfc` without a tag |
| PUT | `/api/v1/nfc` | `{"url": "https://example.com"}`: write a URL that phones open when they touch the tag, at most 240 bytes; `https://` is added when the address has no scheme. Returns what the tag holds afterwards; 409 `not_formatted` when the tag is not NDEF formatted (format it with a phone app, for example NXP TagWriter) *(changes)* |
| GET | `/metrics` | Prometheus text format |

Errors are JSON: `{"error": {"code": "…", "message": "…"}}`.

The latest readings:

```json
{
  "ts": 1792245600000,
  "time": "2026-10-17T16:00:00+02:00",
  "stale": false,
  "readings": [
    { "ts": 1792245600000, "time": "…", "device": "ft260", "channel": 0,
      "temperature": 22.81, "humidity": 41.2, "illuminance": 318.5, "pressure": 100421.0, "altitude": 75.6 },
    { "ts": 1792245600000, "time": "…", "device": "ft260", "channel": 1,
      "temperature": 12.44, "humidity": 70.3, "illuminance": 9120.0, "pressure": 100433.0, "altitude": 74.6 }
  ]
}
```

`device` is always `ft260`: the bridge has no serial number to tell two apart.

The history has a series per channel: `channels` lists them (e.g. `[0, 1]`),
and each metric holds one `{"avg": [...], "min": [...], "max": [...]}` per
entry of `channels`, aligned with `t` (seconds). A channel without a value in
a bucket has `null` there. Readings carry `co2` and `voc` too; they are `null`
without a TOWER CO2 Module or VOC-LP Tag on that channel.

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
`tower_bridge_relay_state{channel="0"} 1` (`true`) or `0` (`false`).

Every reading in `/metrics` carries a `channel` label, for example
`tower_bridge_temperature_celsius{device="ft260",channel="1"} 12.44`;
alert gauges are labelled with the metric, channel and bound. CO₂ and VOC are
`tower_bridge_co2_ppm` and `tower_bridge_voc_ppb`.

Prometheus scrape config:

```yaml
scrape_configs:
  - job_name: tower-bridge
    static_configs:
      - targets: ["127.0.0.1:8180"]
```

## MCP server

The tools use friendly units (°C, %, lx, **hPa**, ppm, ppb):

| Tool | Description |
|---|---|
| `get_current_readings` | Latest values of each channel with the module that measured them, measurement time, active alerts |
| `get_history` | Averaged points and min/avg/max per channel for `hours` back or a `from`/`to` range |
| `get_alerts` | Alert limits, active alerts, recent events (with the channel) |
| `get_device_status` | Bridge status, the modules and sensors on each channel with their I²C addresses, interval, relay state, GPS position, the URL on the NFC Tag |
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

`tower-bridge-cli mcp` does the same and suits a computer without the desktop
app, where `tower-bridge-cli run` is the service ([cli.md](cli.md)):

```json
{
  "mcpServers": {
    "tower-bridge": { "command": "tower-bridge-cli", "args": ["mcp"] }
  }
}
```

### Testing

```sh
npx @modelcontextprotocol/inspector --cli http://127.0.0.1:8180/mcp --transport http --method tools/list
```
