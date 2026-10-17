# Changelog

## 0.1.0 — unreleased

- First version: TOWER Bridge Module + Climate Module (R1, R2) support,
  menu bar / tray readings, dashboard with history charts, alerts with
  notifications, CSV export, optional built-in MQTT broker (topics
  `tower/temperature`, `tower/humidity`, `tower/illuminance`, `tower/pressure`,
  `tower/altitude`), open at login.
- Optional local HTTP server: REST API, Prometheus metrics and an MCP server
  (Streamable HTTP, protocol 2026-07-28 and earlier; `--mcp-stdio` bridge for
  Claude Desktop). Read-only unless "Allow changes" is enabled.
- Theme setting (system, light, dark) and user interface in English, Czech,
  Slovak, German and Polish, including tray menu, notifications and dialogs.
- Interactive charts: min / avg / max values that toggle their lines, value
  and exact time under the pointer, time span selector on the dashboard, drag
  to pan through all stored data, Ctrl/⌘+scroll or pinch to zoom, Shift+drag to
  zoom into a range (with finer data), reset button; line colour per metric
  from a palette that works in light and dark mode.
- Every chart and the relay card name their data source behind an info icon:
  module, I²C bus, switch channel, sensor address and part (e.g. Climate
  Module R1 · I²C (FT260) → channel 0 → 0x48 TMP112).
- Dashboard: up to two charts per row, panels rearranged by dragging them,
  charts can be maximized; tray menu starts with Open Dashboard.
- XLSX export.
- Date format (automatic, ISO, D.M.Y, D/M/Y, M/D/Y) and 12/24-hour time
  settings, used in the dashboard, the tray and XLSX exports. The overview
  shows the absolute time of the last update.
- TOWER Relay Module support (TCA9534A or TCA9534): detected automatically,
  state TRUE / FALSE / unknown (JSON true / false / null), activated or pulsed
  to either state
  (default 2 s) from the
  overview, the REST API (`/api/v1/relay`), MCP (`set_relay`, `pulse_relay`)
  and MQTT (`tower/relay`, `tower/relay/set`, `tower/relay/pulse`);
  `tower-bridge-cli relay`.
- Embedded Inter and Roboto Mono fonts covering all EU languages.
- Requires Rust 1.99; dependencies updated (axum 0.8, rust_xlsxwriter 0.100,
  pollster 1, getrandom 0.4, TypeScript 6).
- JSON keys in the REST API, MQTT payloads, MCP and settings are lowercase
  with underscores (older settings files are converted on load).
