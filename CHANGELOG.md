# Changelog

## 1.1.1 (2026-11-10)

- Alerts: a value that leaves its range again within **Repeat after** of the
  last notification is notified once that time has passed, if it is still out
  of range. Before, such an alert stayed silent however long it lasted.
- Relay: a timed pulse running while the computer sleeps ends at its time
  after the wake, not as much later as the sleep lasted. A pulse is no longer
  cut short, with the relay left switched over, when the sensors stop
  answering and the bridge is opened again.
- The desktop app quits as with **Quit** when the system or `kill` ends it
  (SIGTERM, SIGINT, SIGHUP on macOS and Linux): a running relay pulse is
  switched back and the sensors' state is saved. Its log keeps its last lines
  and records crashes of its background threads.
- MQTT: when the app or `run` stops during a relay pulse, the broker gets the
  switched-back state before `offline`. A broker that accepts the connection
  and then drops it (the same client ID in use elsewhere, for example) is
  tried again after a growing delay, not every second.
- Settings: a setting written by a newer version of TOWER Bridge is kept when
  an older one (a CLI next to a newer app, for example) saves the file.
- Local API: one address on the network holds at most 16 of the 64
  connections, and a request that is not complete 30 seconds after its first
  byte is closed, so a host on the network cannot block the API without the
  token. MCP requests need a string or integer `id`; the stdio bridge answers
  a batch with an error instead of nothing.
- NFC Tag: spaces, line breaks and other control characters in a URL are
  percent-encoded, when read (a phone may have written anything) and when
  written.
- Charts: zoomed to a few days, the time axis shows the date once a day and
  the time at the other ticks, instead of one date several times. Panning a
  zoomed chart no longer stops at an invisible edge while data loads. A value
  just below zero shows as 0.0, not -0.0.
- Dashboard: Escape in a chart's colour palette closes only the palette, not
  a maximized card. The palette and the theme buttons in the sidebar take the
  arrow keys, and screen readers read the legend's values with their names.
  The Data page offers no export while a custom date is incomplete.
- GPS Module: a receiver that missed its setup at power-up gets it again (it
  kept sending NMEA, which held the bus for seconds every 5 s).
- A Humidity Tag with an HDC2080 is no longer taken for one with an SHT20 (it
  showed -46.85 °C); the HDC2080 is not supported.
- Updates: a release candidate published without GitHub's pre-release mark is
  not offered, and an answer that cannot be read is tried again the next day,
  not every hour.
- Open at Login: a development build never takes the login item over, also
  when the program that turned it on is gone.
- The tray's MQTT item no longer waits for a settings save on the main thread,
  which could freeze the tray.
- CLI: `config set api.token` refuses a token shorter than 16 characters
  instead of storing a random one. Ctrl+C ends `relay pulse` with exit code
  130 and one JSON object, and stops `gps` at once. The dashboard following
  another instance shows that instance's error (a refused token, say) instead
  of "no answer".
- Dependencies: Tauri 2.12.5, rusqlite 0.41 with SQLite 3.54
  (RUSTSEC-2026-0459), rustls-webpki 0.103.16 (RUSTSEC-2026-0382; MQTT over TLS
  and the update check), and other updates.
- Releases: the Windows signing step uses no cached tools, the draft release
  takes exactly the expected files, and a weekly audit checks the
  dependencies for new advisories.

## 1.1.0 (2026-10-23)

- Update notices: once a day the app asks GitHub for the newest release
  (nothing else is sent). When a newer version is out, the tray menu starts
  with **Update Available**, the Dashboard shows a banner with **Download**,
  and a notification says so once per version. The app does not install
  anything itself; the manual's Updates chapter says how to put the new
  version in place. *Settings → About* shows the result of the last check,
  has **Check now**, and switches the daily check off (**Check for updates**,
  on by default).

## 1.0.1 (2026-10-19)

- macOS: notifications work again. They go through the system's current
  notification framework, which 1.0.0's notification library no longer
  reached on current macOS (nothing appeared, not even a permission request).
  TOWER Bridge asks for permission when it first starts, shows notifications
  also while the dashboard is in front, and opens the dashboard when one is
  clicked. If notifications are turned off, **Send test** in Alerts says so
  and **Open settings** opens the place to turn them on.
- macOS 13 and newer: **Open at Login** registers TOWER Bridge itself as a
  login item, listed by name and icon under Open at Login in System Settings,
  instead of a background item listed as "HARDWARIO a.s.". The launch agent
  of 1.0.0 is replaced when the app starts.

## 1.0.0 (2026-10-19)

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
  to 1 hour (2 s by default); a pulse running when the app or the service
  stops is switched back.
- TOWER GPS Module (u-blox SAM-M8Q): fix, satellites, position, accuracy,
  altitude and satellite time; shown as searching about a second after the
  receiver powers up, although its first report can take two minutes.
- TOWER NFC Tag: shows the URL on the tag and writes a new one as an NDEF URI
  record that phones open when they touch the tag.
- More than one TOWER Bridge Module plugged in: the card and the tray say so
  (only one is read; Identify shows which).

### App

- Menu bar or tray icon with a value of your choice, and the readings of every
  sensor in its menu.
- The HARDWARIO TOWER logo at the top of the sidebar (its symbol on the
  folded rail), white where it is grey in dark mode.
- The dashboard window opens tall enough for the whole sidebar (820 px, or
  what the screen has room for).
- Dashboard: a panel per measurement with a line per channel, min, avg and max
  that toggle their lines, value and time under the pointer, time spans from
  1 hour to 365 days, drag to pan through all stored data, Ctrl/⌘+scroll,
  pinch or Shift+drag to zoom, line colours from a palette that works in light
  and dark mode, alert limits as a light red band, lines hidden from the
  legend, panels rearranged by dragging and maximized. Spans of weeks to a
  year load at once from an hourly summary. The measurement interval (5 s to 3 min, 20 s by default) is
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
  diagrams that follow the theme, and the third-party notices (every library,
  font, icon set and map source with its license).
- One file per platform: a portable Windows exe, a macOS dmg, a Linux
  AppImage. The Windows exe and the Windows command line tool are signed by
  HARDWARIO a.s. (Authenticode, timestamped); the macOS app, its dmg and the
  macOS command line tool are signed with Developer ID and notarized.

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
  milliseconds (`_ms`).
- The REST API reads and writes the NFC Tag (`/api/v1/nfc`) and reports the
  MQTT connection in `/api/v1/status`.

### Command line

- `tower-bridge-cli`, the headless sibling of the app, sharing its settings
  and history: a terminal dashboard (values with charts, relay, GPS, NFC,
  alerts, log); `status`, `watch`, `relay`, `gps`, `nfc`, `identify`,
  `rescan`, `export`, `alerts` and `config` as text or JSON; `run`, the
  service without a window (history, alerts, MQTT, REST, MCP) for a
  Raspberry Pi or a server; `mcp`; and the bring-up tools under `toolbox`.
  While the app or the service holds the bridge, the commands go through its
  API server. Documented exit codes; release archives for Linux (x86_64 and
  64-bit ARM), macOS and Windows.
- One instance per data folder: the app says so when `tower-bridge-cli run`
  already holds it.
