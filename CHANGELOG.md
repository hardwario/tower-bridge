# Changelog

## 0.1.0 (unreleased)

- First version: TOWER Bridge Module + Climate Module (R1, R2) support,
  menu bar / tray readings, dashboard with history charts, alerts with
  notifications, CSV export, optional MQTT (topics
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
- Built-in user manual (Manual in the sidebar) in all five languages, with
  screenshots and diagrams that follow the theme; written in `docs/manual`.
- `tower-bridge-cli i2c-identify` names the sensor chips found on an I²C
  channel.
- Sensors on both I²C channels: besides the Climate Module, the Humidity Tag
  (SHT20), Temperature Tag (TMP112), Lux Meter Tag (OPT3001) and Barometer Tag
  (MPL3115A2) are found part by part, on either channel, also when plugged in
  while the app runs, including the tags' alternative addresses (Temperature
  Tag 0x49, Lux Meter Tag 0x45). Each panel shows a line, a colour and min / avg / max
  per channel; readings, exports, alerts, REST, MCP and Prometheus metrics
  carry the channel. Existing history becomes channel 0.
- Robustness, from a full review:
  - MQTT ignores commands with the retain flag, so a stored command never
    fires later; it clears the topics of a removed channel or an old prefix,
    and rejects pulse payloads with unknown fields.
  - Alerts of a removed channel end, and active alerts follow the readings.
  - Notifications name the channel when both channels carry the same module.
  - "Reconnected" only follows a shown "disconnected"; missing sensors no
    longer count as a USB disconnect.
  - A settings file with an unknown value (e.g. from a newer version) keeps
    all the other settings.
  - Concurrent settings changes cannot overwrite each other.
  - REST errors are always JSON; a rescan no longer stalls other requests.
  - The schema version is written in the same transaction as the schema.
  - The relay stays responsive during a barometer reset.
- MQTT goes to your own broker: the app connects as a client to Mosquitto,
  Home Assistant or any MQTT broker (TLS and login optional), reconnects by
  itself and publishes `tower/status` (`online`, and `offline` as its last
  will). Settings apply without a restart. The built-in broker (rumqttd) is
  gone; a settings file that used it turns MQTT off.
- Identify no longer shows a message when the LED blinked (only when it
  failed).
- More than one TOWER Bridge Module plugged in: the Bridge Module card and the
  tray say so (only one is read; Identify shows which). The bridge's serial
  number is no longer shown (the FT260 has none).
- The Data table sorts by any column: click a column head, again to reverse.
- Decimal separator setting (Settings, General): Automatic follows the
  language, or a point or a comma, for numbers in the app, the tray and
  notifications. Exports are not affected.
- The drag handle is the rightmost tool in every panel header.
- The measurement interval is set in the Dashboard header, next to the last
  update (5 s, 10 s, 20 s, 1 min, 3 min), instead of Settings; the default is now 20 s, and
  settings still on the old 10 s default move to it.
- The relay card's buttons keep their width when clicked (the current state
  is no longer set in bold).
- The History page is now the Data page: the readings as a table for the
  Dashboard's time spans (1 hour to 365 days) or custom dates, with CSV and
  XLSX export. Charts are on the
  Dashboard only.
- Manual: jumping to a chapter lands on it (screenshots no longer push it
  away), and the contents no longer flicker on the way.
- Click a line's name in a chart legend to hide or show it; the legend shows a
  hidden line in grey, and one line always stays.
- The TOWER Bridge Module card lists one module per line after its I²C
  channel, by full product name (TOWER Climate Module, with the revision set
  small), the Relay Module included. The connection column is gone.
- MQTT topics end with the I²C channel: `tower/temperature/0`,
  `tower/humidity/1`, …; the relay is `tower/relay/<channel>` with `/set` and
  `/pulse` below it.
