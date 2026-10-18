# TOWER Bridge

Desktop app for the HARDWARIO **TOWER Bridge Module** with a **TOWER Climate
Module** or TOWER sensor tags (Humidity, Temperature, Lux Meter, Barometer) on
either of its two I²C channels and, optionally, a **TOWER CO2 Module**, a
**TOWER VOC-LP Tag**, a **TOWER GPS Module**, a **TOWER Relay Module** and a **TOWER NFC
Tag**. It lives in the menu bar (macOS) or system tray (Windows, Linux) and
shows temperature, humidity, illuminance, barometric pressure, CO₂ and VOC at a
glance.

- Current value right in the menu bar / tray icon, all readings in the tray menu
- Dashboard with live values and interactive charts: min / avg / max with
  optional lines, exact value and time under the pointer, selectable time
  span, drag to pan through all stored data, Ctrl/⌘+scroll, pinch or
  Shift+drag to zoom, line colors from an 8-color palette, history from
  1 h to 365 days or a custom range, and a table view; dashboard panels can be
  rearranged by dragging and charts maximized; with sensors on both channels,
  each panel shows a line per channel
- Relay Module control (true / false / timed pulse) on the dashboard, over
  REST, MCP and MQTT when one is connected
- CO₂ (TOWER CO2 Module, pressure compensated) and TVOC (VOC-LP Tag, humidity
  compensated) as full measurements: panels, alerts, export, MQTT, REST and MCP
- TOWER GPS Module: fix, position on an offline map (and a link to
  OpenStreetMap), altitude and satellites on the dashboard, over MQTT, REST
  and MCP
- TOWER NFC Tag: write a URL (an NDEF URI record) that phones open when they touch
  the tag
- Notifications when a value leaves its range (with hysteresis and repeat
  interval), and when the bridge is disconnected
- CSV and XLSX export of the stored history
- Optional MQTT to your own broker (Mosquitto, Home Assistant), publishing
  every reading with its I²C channel
  (`tower/temperature/0`, `tower/humidity/1`, …) and the app's availability
- Optional local REST API, Prometheus metrics and MCP server, so scripts,
  Grafana and AI assistants (Claude, Cursor, VS Code) can use the readings
- Open at login
- Light or dark theme, or follow the system; date and 12/24-hour time format
- English, Čeština, Slovenčina, Deutsch and Polski (follows the system language
  by default), including the tray menu and notifications
- One file per platform, no installer on Windows
- `tower-bridge-cli`, the headless sibling: a terminal dashboard, commands for
  scripts (text or JSON), and a service without a window for a Raspberry Pi
  or a server, sharing the app's settings and history ([docs/cli.md](docs/cli.md))

It continues [hardwario/enmon](https://github.com/hardwario/enmon), the
command-line reader for the same hardware.

## Download

| Platform | File |
|---|---|
| Windows 10/11 (x64) | `TOWER-Bridge-<version>-windows-x64-portable.exe` |
| macOS 11+ with Safari 16.2+ (Apple silicon and Intel) | `TOWER-Bridge-<version>-macos-universal.dmg` |
| Linux (x86_64) | `TOWER-Bridge-<version>-linux-x86_64.AppImage` |
| Command line, every platform (Linux also on 64-bit ARM) | `tower-bridge-cli-<version>-<target>.tar.gz` (`.zip` on Windows), see [docs/cli.md](docs/cli.md) |

### First run

- **Windows:** run the exe; no installation needed. The dashboard needs the
  Microsoft Edge WebView2 Runtime, which ships with Windows 11 and current
  Windows 10. Builds are not code-signed yet: if SmartScreen warns, choose
  *More info → Run anyway*. Windows 11 may hide new tray icons in the overflow
  area; drag the icon to the taskbar to keep it visible.
- **macOS:** open the dmg and drag *TOWER Bridge* to *Applications* (Open at
  Login requires the app to be in Applications). The window is drawn by the
  system's WebKit, so keep Safari updated (16.2 or newer; Software Update
  offers it for macOS 11 too). Unsigned builds must be
  allowed once in *System Settings → Privacy & Security → Open Anyway*.
- **Linux:** install the udev rule from `packaging/linux/` and see
  [docs/linux.md](docs/linux.md) (kernel driver notes, GNOME tray extension).

### Portable mode (Windows)

Create a folder named `data` next to the exe: settings, history and logs are
then stored there instead of `%LOCALAPPDATA%`, so the app can run from a USB
stick. The only thing written outside is a registry key that lets Windows show
notifications (`HKCU\Software\Classes\AppUserModelId\com.hardwario.tower-bridge`).

## Using it

- The menu bar / tray shows the value chosen in *Settings → Value in menu bar*.
  Click the icon for all readings and quick toggles; on Windows a left click
  opens the dashboard.
- Closing the dashboard keeps the app running in the tray. Quit from the tray
  menu.
- Readings are stored in SQLite (default retention 90 days) in the data folder
  shown in *Settings → Data*.
- MQTT is described in [docs/mqtt.md](docs/mqtt.md), the REST API,
  metrics and MCP server in [docs/api.md](docs/api.md).
- Without a desktop, `tower-bridge-cli run` does what the app does, and
  `tower-bridge-cli` alone opens a terminal dashboard ([docs/cli.md](docs/cli.md)).
  Only one of them runs per data folder; the other commands then go through
  its API server.
- The user manual is built in: open *Manual* in the sidebar. Its source, in all
  five languages, is in [docs/manual](docs/manual/en.md).

## Building from source

Requirements: Rust 1.99+, Node.js 22+, and the
[Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS
(on Linux: `libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev
libudev-dev`).

```sh
npm ci
npm run tauri dev                         # run with hot reload
TOWER_BRIDGE_SIMULATE=1 npm run tauri dev # no hardware: simulated Climate Module and tags
npm run tauri build                       # release bundles (dmg / AppImage)
npm run tauri build -- --no-bundle        # Windows portable exe
```

A debug binary started directly (`cargo run -p tower-bridge`) loads the
dashboard from the Vite dev server at `localhost:1420` and shows a blank window
without it. Use `npm run tauri dev`, or embed the dashboard:
`npm run build && cargo run -p tower-bridge --features tauri/custom-protocol`.

Development helpers:

| Variable | Effect |
|---|---|
| `TOWER_BRIDGE_SIMULATE=1` (or `--simulate`) | Use a simulated bridge; data goes to a separate `simulated` folder. Runs alongside an installed copy (no single-instance check) |
| `TOWER_BRIDGE_SEED_DAYS=14` | With simulation, fill an empty history with generated data |
| `TOWER_BRIDGE_DATA_DIR=/path` | Override the data folder (also skips the single-instance check) |
| `TOWER_BRIDGE_LOG=debug` | Log level (logs also go to the data folder) |

`npm run dev` and opening `http://localhost:1420/preview.html` runs the
dashboard in a browser against a mocked backend.

`npm run manual:screenshots` regenerates the screenshots of the user manual
from that preview, in every language and theme (needs Google Chrome).

Translations: the dashboard dictionaries are `src/lib/i18n/{en,cs,sk,de,pl}.ts`
(English is the reference; the others are type-checked against it). Tray
menu, notification and dialog texts are in `src-tauri/src/i18n.rs`.

Tests and checks:

```sh
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features
npm run check && npm test
npm run test:ui          # browser smoke test of every view (needs Google Chrome)
npm run test:ui:webkit   # the same in WebKit (npx playwright-core install webkit)
```

## Project layout

| Path | Contents |
|---|---|
| `crates/tower-bridge-core` | FT260 transport (hidapi, Linux i2c-dev), sensor drivers, sensor detection on both I²C channels, acquisition thread, alert engine, SQLite store, MQTT client, REST/MCP server |
| `crates/tower-bridge-service` | The service the app and the CLI share: data folder and settings, the hub (storage, alerts, MQTT), commands to the bridge, the API server; one instance per data folder |
| `crates/tower-bridge-cli` | `tower-bridge-cli`: terminal dashboard, commands, the service without a window (`run`), MCP bridge, and the bring-up `toolbox` |
| `src-tauri` | Desktop app: tray, dashboard window, notifications, Tauri commands |
| `src` | Dashboard UI (Svelte 5, uPlot); translations in `src/lib/i18n/` |
| `docs` | [Command line](docs/cli.md), [Hardware protocol](docs/hardware.md), [Linux](docs/linux.md), [MQTT](docs/mqtt.md), [REST API & MCP](docs/api.md), [user manual](docs/manual/en.md) |
| `packaging/linux` | udev rule |

## Contributing

Read [AGENTS.md](AGENTS.md) first: architecture, commands, conventions and
lessons learned, for people and coding agents alike. Work in progress is listed
in [TODO.md](TODO.md).

## License

MIT, © 2026 HARDWARIO a.s. See [LICENSE](LICENSE).

The MIT license covers the source code. The HARDWARIO and TOWER names and the
HARDWARIO logo are trademarks of HARDWARIO a.s. and are not licensed for use in
derived products.

Third-party components include [Tauri](https://tauri.app) (MIT/Apache-2.0),
[hidapi](https://github.com/libusb/hidapi) (BSD-3-Clause),
[rumqttc](https://github.com/bytebeamio/rumqtt) (Apache-2.0),
[axum](https://github.com/tokio-rs/axum) (MIT),
[SQLite](https://sqlite.org) (public domain), [uPlot](https://github.com/leeoniya/uPlot)
(MIT), [Svelte](https://svelte.dev) (MIT), icons from
[Lucide](https://lucide.dev) (ISC), the [Inter](https://rsms.me/inter/) and
[Roboto Mono](https://github.com/googlefonts/RobotoMono) fonts (SIL OFL 1.1,
`src/assets/fonts/OFL-*.txt`), the Barlow Condensed font (SIL OFL 1.1,
`src-tauri/assets/fonts/OFL.txt`), and map data from
[Natural Earth](https://www.naturalearthdata.com) (public domain) via
[world-atlas](https://github.com/topojson/world-atlas) (ISC) with place names
from [Wikidata](https://www.wikidata.org) (CC0).

[THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md) lists every crate, npm package,
font, icon set and map source in the app with its license text; the app shows it
from the About chapter of the Manual. Regenerate it with `npm run notices` after
a dependency change (CI fails when it is out of date).
