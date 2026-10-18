# Changelog

## 0.1.0 (unreleased)

The first version: a tray app for the HARDWARIO TOWER Bridge Module (FTDI
FT260) on macOS, Windows and Linux, continuing the command line tool
hardwario/enmon.

### Hardware

- TOWER Climate Module R1 and R2, and the Humidity, Temperature, Lux Meter and
  Barometer Tags, on either I²C channel of the bridge, found part by part,
  also when plugged in while the app runs, including the tags' alternative
  addresses (Temperature Tag 0x49, Lux Meter Tag 0x45).
- TOWER CO2 Module (Senseair LP8): CO₂ in ppm, filtered and pressure
  compensated with the barometer, at most every 16 s. The sensor's state and
  its automatic baseline correction survive restarts.
- VOC-LP Tag (Sensirion SGPC3 in ultra-low-power mode): TVOC in ppb every
  30 s, humidity compensated; the learned baseline survives restarts.
- TOWER Relay Module (TCA9534A or TCA9534): state TRUE, FALSE or unknown
  (JSON `true`, `false`, `null`), switched or pulsed to either state for 0.5 s
  to 1 hour (2 s by default).
- TOWER GPS Module (u-blox SAM-M8Q): fix, satellites, position, accuracy,
  altitude and satellite time.
- TOWER NFC Tag: shows the URL on the tag and writes a new one as an NDEF URI
  record that phones open when they touch the tag.
- More than one TOWER Bridge Module plugged in: the card and the tray say so
  (only one is read; Identify shows which).

### App

- Menu bar or tray icon with a value of your choice, and the readings of every
  sensor in its menu.
- Dashboard: a panel per measurement with a line per channel, min, avg and max
  that toggle their lines, value and time under the pointer, time spans from
  1 hour to 365 days, drag to pan through all stored data, Ctrl/⌘+scroll,
  pinch or Shift+drag to zoom, line colours from a palette that works in light
  and dark mode, lines hidden from the legend, panels rearranged by dragging
  and maximized. The measurement interval (5 s to 3 min, 20 s by default) is
  set in its header.
- Every chart and card names its data source: module, I²C channel, address
  and part.
- Relay card with Manual control and Pulse control rows; the running pulse
  fills its button and ends early on a second click.
- GPS card with an offline map (Natural Earth countries, rivers, lakes and
  towns, the town names in the app's language) and a link to OpenStreetMap.
- NFC card to read and write the tag's URL.
- Data page: the readings as a sortable table for the Dashboard's time spans
  or custom dates, CSV and XLSX export.
- Alerts with limits, hysteresis and a repeat interval for every measurement,
  notifications when a value leaves its range and when it is back, and when
  the bridge is disconnected.
- Settings for theme (system, light, dark), language (English, Czech, Slovak,
  German, Polish), date and time format, decimal separator, °C or °F, the
  tray value, open at login, the sea-level pressure for the altitude
  estimate, the I²C channels, data retention.
- A user manual in all five languages inside the app, with screenshots and
  diagrams that follow the theme.
- One file per platform: a portable Windows exe, a macOS dmg, a Linux
  AppImage.

### Integrations

- MQTT client for your broker (Mosquitto, Home Assistant; TLS and login
  optional): every reading as a retained value on `tower/<metric>/<channel>`
  (`temperature`, `humidity`, `illuminance`, `pressure`, `altitude`, `co2`,
  `voc`), the GPS position on `tower/gps/<channel>/latitude`, `/longitude`,
  `/altitude` and `/satellites`, the relay on `tower/relay/<channel>` with
  `/set` and `/pulse` commands when allowed, and `tower/status` (`online`,
  and `offline` as the last will).
- Local HTTP server, off by default: a REST API with an OpenAPI description,
  Prometheus metrics and an MCP server for AI assistants (Streamable HTTP,
  protocol 2026-07-28 and earlier, and a `--mcp-stdio` bridge). Read-only
  unless "Allow changes" is on; network access needs a token.
- JSON keys are lowercase with underscores everywhere; durations are integer
  milliseconds (`_ms`), except `cooldown_s` and `sample_interval_s`.

### Command line

- `tower-bridge-cli` for bringing up hardware: `scan`, `probe`, `watch`,
  `led`, `i2c-scan`, `i2c-identify`, `i2c-read`, `i2c-write`, `relay`, `co2`,
  `voc`, `gps`, `nfc`, each also with a simulated bridge (`--simulate`).
