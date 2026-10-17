# TOWER Bridge

Desktop app for the HARDWARIO **TOWER Bridge Module** with a **TOWER Climate
Module** and, optionally, a **TOWER Relay Module**. It lives in the menu bar (macOS) or system tray (Windows, Linux) and
shows temperature, humidity, illuminance and barometric pressure at a glance.

- Current value right in the menu bar / tray icon, all readings in the tray menu
- Dashboard with live values and interactive charts: min / avg / max with
  optional lines, exact value and time under the pointer, selectable time
  span, drag to pan through all stored data, Ctrl/⌘+scroll, pinch or
  Shift+drag to zoom, line colours from an 8-colour palette, history from
  1 h to 30 days or a custom range, and a table view; dashboard panels can be
  rearranged by dragging and charts maximized
- Relay Module control (true / false / timed pulse) on the dashboard, over
  REST, MCP and MQTT when one is connected
- Notifications when a value leaves its range (with hysteresis and repeat
  interval), and when the bridge is disconnected
- CSV and XLSX export of the stored history
- Optional built-in MQTT broker publishing every reading (`tower/temperature`, `tower/humidity`, …)
- Optional local REST API, Prometheus metrics and MCP server, so scripts,
  Grafana and AI assistants (Claude, Cursor, VS Code) can use the readings
- Open at login
- Light or dark theme, or follow the system; date and 12/24-hour time format
- English, Čeština, Slovenčina, Deutsch and Polski (follows the system language
  by default), including the tray menu and notifications
- One file per platform, no installer on Windows

It continues [hardwario/enmon](https://github.com/hardwario/enmon), the
command-line reader for the same hardware.

## Download

| Platform | File |
|---|---|
| Windows 10/11 (x64) | `TOWER-Bridge-<version>-windows-x64-portable.exe` |
| macOS 11+ (Apple silicon and Intel) | `TOWER-Bridge-<version>-macos-universal.dmg` |
| Linux (x86_64) | `TOWER-Bridge-<version>-linux-x86_64.AppImage` |

### First run

- **Windows:** run the exe; no installation needed. The dashboard needs the
  Microsoft Edge WebView2 Runtime, which ships with Windows 11 and current
  Windows 10. Builds are not code-signed yet: if SmartScreen warns, choose
  *More info → Run anyway*. Windows 11 may hide new tray icons in the overflow
  area; drag the icon to the taskbar to keep it visible.
- **macOS:** open the dmg and drag *TOWER Bridge* to *Applications* (Open at
  Login requires the app to be in Applications). Unsigned builds must be
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
- The MQTT broker is described in [docs/mqtt.md](docs/mqtt.md), the REST API,
  metrics and MCP server in [docs/api.md](docs/api.md).

## Building from source

Requirements: Rust 1.99+, Node.js 22+, and the
[Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS
(on Linux: `libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev
libudev-dev`).

```sh
npm ci
npm run tauri dev                         # run with hot reload
TOWER_BRIDGE_SIMULATE=1 npm run tauri dev # no hardware: simulated Climate Module
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

Translations: the dashboard dictionaries are `src/lib/i18n/{en,cs,sk,de,pl}.ts`
(English is the reference; the others are type-checked against it). Tray
menu, notification and dialog texts are in `src-tauri/src/i18n.rs`.

Tests and checks:

```sh
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features
npm run check && npm test
```

## Project layout

| Path | Contents |
|---|---|
| `crates/tower-bridge-core` | FT260 transport (hidapi, Linux i2c-dev), sensor drivers, Climate Module logic, acquisition thread, alert engine, SQLite store, MQTT broker, REST/MCP server |
| `crates/tower-bridge-cli` | `tower-bridge-cli` diagnostics tool (`scan`, `probe`, `watch`, `led`, `i2c-scan`) |
| `src-tauri` | Desktop app: tray, dashboard window, settings, notifications, commands |
| `src` | Dashboard UI (Svelte 5, uPlot); translations in `src/lib/i18n/` |
| `docs` | [Hardware protocol](docs/hardware.md), [Linux](docs/linux.md), [MQTT](docs/mqtt.md), [REST API & MCP](docs/api.md) |
| `packaging/linux` | udev rule |

## License

MIT, © 2026 HARDWARIO a.s. — see [LICENSE](LICENSE).

The MIT license covers the source code. The HARDWARIO and TOWER names and the
HARDWARIO logo are trademarks of HARDWARIO a.s. and are not licensed for use in
derived products.

Third-party components include [Tauri](https://tauri.app) (MIT/Apache-2.0),
[hidapi](https://github.com/libusb/hidapi) (BSD-3-Clause),
[rumqttd](https://github.com/bytebeamio/rumqtt) (Apache-2.0),
[axum](https://github.com/tokio-rs/axum) (MIT),
[SQLite](https://sqlite.org) (public domain), [uPlot](https://github.com/leeoniya/uPlot)
(MIT), [Svelte](https://svelte.dev) (MIT), icons from
[Lucide](https://lucide.dev) (ISC), the [Inter](https://rsms.me/inter/) and
[Roboto Mono](https://github.com/googlefonts/RobotoMono) fonts (SIL OFL 1.1,
`src/assets/fonts/OFL-*.txt`) and the Barlow Condensed font (SIL OFL 1.1,
`src-tauri/assets/fonts/OFL.txt`).
