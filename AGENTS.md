# AGENTS.md

Working notes for everyone who changes this repository, people and coding agents alike. This file covers:

- the architecture,
- the commands,
- the conventions,
- the rules for working safely next to real hardware,
- what earlier sessions learned the hard way.

Keep it current. If a change makes a statement here wrong, fix the statement in the same change. If you learn something non-obvious, add it to [Key learnings](#key-learnings).

Open and ongoing work is tracked in [TODO.md](TODO.md); released changes go in [CHANGELOG.md](CHANGELOG.md).

## What this is

TOWER Bridge is a tray-first desktop app (Tauri 2, Rust, Svelte 5) for the HARDWARIO TOWER Bridge Module, an FTDI FT260 USB-HID to I²C bridge. It:

- reads the TOWER Climate Module, the TOWER sensor tags (temperature, humidity, illuminance, pressure), the TOWER CO2 Module and the VOC-LP Tag on both I²C channels of the bridge;
- switches the TOWER Relay Module, shows the TOWER GPS Module's position on an offline map, and writes a URL to the TOWER NFC Tag;
- stores the history in SQLite;
- shows charts and alerts;
- offers MQTT, REST, Prometheus metrics and MCP.

It continues the CLI tool hardwario/enmon. MIT license, © HARDWARIO a.s.

## Commands

Prerequisites: Rust 1.99+, Node 22+. On Linux, see the CI workflow for the apt packages.

```sh
npm ci                                       # once
npm run dev                                  # browser preview, open http://localhost:1420/preview.html
npm run tauri dev                            # the real app with hot reload
TOWER_BRIDGE_SIMULATE=1 npm run tauri dev    # no hardware needed: a simulated bridge with every module
                                             # (Climate Module, tags, relay, CO2, VOC-LP, GPS, NFC)
cargo run -p tower-bridge-cli -- --simulate  # the terminal dashboard on the simulated bridge
cargo run -p tower-bridge-cli -- --help      # status, watch, relay, gps, nfc, export, config, run, toolbox, ...
```

A debug binary that embeds the built frontend and needs no dev server:

```sh
npm run build && cargo build -p tower-bridge --features tauri/custom-protocol
./target/debug/tower-bridge --simulate
```

Run all of these before handing work over; CI runs the same:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
npm run check
npm test
npm run test:ui          # every view and device state in the browser preview, in Chrome
npm run test:ui:webkit   # the same in WebKit, which macOS and Linux render the app with
                         # (once: npx playwright-core install webkit)
npm run build
```

After a visible UI change, regenerate the manual's screenshots with `npm run manual:screenshots`. It needs Google Chrome and takes about a minute.

After adding, removing or updating a dependency (Cargo or npm), regenerate the third-party notices with `npm run notices`; CI runs `npm run notices -- --check`.

### The browser preview

`preview.html` runs the whole UI in a browser with a mocked backend (`src/preview.ts`). Query parameters select the situation, for example `?tab=manual&lang=de&theme=dark&relay=unknown`, or `tags=0` for a Climate Module alone. The full list is at the top of `src/preview.ts`. `scripts/ui-smoke.mjs` (`npm run test:ui`) walks through it and fails on errors; add a case there for every new view or state.

Use it with headless Chrome or Playwright to check layouts in every language and theme before touching the real app. `scripts/manual-screenshots.mjs` is a worked example.

## Architecture

```
crates/tower-bridge-core/   hardware, data and protocols; no Tauri dependency
  bus/                      I2cBus trait: FT260 over hidapi, Linux i2c-dev, mock
  sensors/                  SHT30, SHT20, TMP112, OPT3001, MPL3115A2, SGPC3, LP8 frames, SC16IS740 UART, CRC, identify (I²C part detection)
  climate.rs                I²C switch, sensor detection per channel and part, measurement cycles
  relay.rs                  Relay Module (TCA9534A/TCA9534 driving a latching relay)
  nfc.rs                    NFC Tag (NTAG I²C plus): NDEF URI read and write
  co2.rs                    CO2 Module: LP8 power cycle, capacitor, state kept between measurements and runs
  gps.rs                    GPS Module: UBX configuration, NAV-PVT polling
  acquisition.rs            the thread that owns the device: reconnects, measures, runs commands
  alerts.rs                 pure alert engine (limits, hysteresis, repeat interval)
  store/                    SQLite schema, bucketed history, retention, CSV and XLSX export
  mqtt.rs                   client of the user's broker (rumqttc), publishing, relay commands
  api/                      REST, OpenAPI, MCP (HTTP and stdio), security
crates/tower-bridge-service/ the service the app and the CLI share; no Tauri dependency
  service.rs                start and stop: instance lock, settings, store, acquisition, hub, API (Mode::Full or Session)
  hub.rs                    fans out acquisition events to the store, alerts, MQTT and the Observer
  observer.rs               what the service reports to its front end (app: webview, tray, notifications)
  control.rs                actions shared by the front ends, REST, MCP and MQTT; settings changes
  state.rs, api.rs          shared state; the API server and its backend; the MCP stdio bridge
  settings.rs, paths.rs     settings.json with versioned migrations; the data folder
  lock.rs                   one running instance per data folder (tower-bridge.lock)
crates/tower-bridge-cli/    tower-bridge-cli: the headless sibling (docs/cli.md)
  access.rs, remote.rs      reach the bridge: the service here, or a running instance through its API
  cmd/                      the commands; toolbox.rs holds the bring-up tools
  tui/                      the terminal dashboard (ratatui): app.rs state and keys, ui.rs drawing, tests.rs
src-tauri/                  the desktop app (package "tower-bridge")
  lib.rs                    startup, plugins, single instance, flags
  observer.rs               the service's events as webview events, tray updates and notifications
  state.rs                  AppState: the Service plus the tray, dereferencing to the service state
  commands.rs               Tauri commands for the webview
  tray/, window.rs          tray menu and icon, the dashboard window
  i18n.rs                   tray and notification texts
src/                        Svelte 5 frontend, no router
  App.svelte                shell: sidebar navigation, tabs
  views/                    Overview (the Dashboard), Data (table and export), Alerts, Settings, Manual
  lib/live.svelte.ts        app state and backend events
  lib/chart-group.svelte.ts shared zoom and pan of the charts on a page
  lib/i18n/                 UI texts in en, cs, sk, de, pl
  lib/manual.ts             renders docs/manual for the Manual tab
  lib/map.ts                offline map of the GPS card: TopoJSON decoding, Web Mercator, canvas drawing
  assets/map/world.json     the map data (Natural Earth 1:10m), built by scripts/build-map.mjs
docs/                       cli.md, api.md, mqtt.md, hardware.md, linux.md, manual/
```

### Data flow

```
FT260 ─USB HID─> bus ─> acquisition thread ──AcqEvent──> hub thread ─┬─> ServiceState (latest values)
                          ▲                                          ├─> SQLite writer
                          │ AcqCommand                               ├─> alert engine
                          │                                          ├─> MQTT publisher
  front ends / REST / MCP / MQTT ─> control.rs                       └─> Observer: app (webview events,
                                                                         tray, notifications) or CLI
                                                                         (dashboard, log lines)
```

- **Several bridges.** Only the first bridge found is opened. The acquisition thread counts the plugged-in bridges (`Bridges::count`, on its own thread, since it owns the HID access) when one opens, every minute and on Rescan, and reports `DeviceInfo.bridges_connected`; the card and the tray warn above one.
- **One instance per data folder.** The desktop app, `tower-bridge-cli run` and the CLI dashboard each run the service (`tower-bridge-service`) and take the data folder's lock (`tower-bridge.lock`, an OS file lock that dies with the process). Short CLI commands run a `Mode::Session` (nothing recorded, published or served, settings only read) and take no lock, so the app starting at login next to a script waits for the bridge instead of refusing. When the lock is held, CLI commands go through the holder's API server, or stop with exit code 3 when it is off; `config set` always refuses then, since the holder would overwrite the file.
- **Front ends through an Observer.** The hub reports to an `Observer` after updating the state: the app turns it into webview events, the tray and notifications (`src-tauri/src/observer.rs`), the CLI into its dashboard or log lines. Notification texts and translations stay in the app.
- **One device owner.** Only the acquisition thread touches the bus. Everything else sends an `AcqCommand` over a channel; replies come back on a one-shot channel. This covers identify, rescan, interval and channel changes, and relay set and pulse.
- **Sensors on two channels.** The bridge's I²C switch has channels 0 and 1, and both may carry sensors (for example a Climate Module on 0 and tags on 1). `climate::Sensors` probes each part on each channel and names the module from the parts found. Every reading, sensor source, alert, export row and history series carries its channel; channels measured in one cycle share the timestamp. Parts are probed again every minute, and a part that fails three times in a row is dropped.
- **Measurement cycle.** A channel takes about 1 s because its sensors convert in parallel; channels are measured one after the other. `ClimateModule::measure_with` calls an idle hook while it waits, so relay commands run within about 25 ms instead of after the cycle. When the hook used the bus, the cycle re-selects the I²C switch channel.
- **Modules on their own clock.** The CO2 Module measures at the end of its channel's cycle when due (at most every 16 s, through the same idle hook), the SGPC3 every 30 s on a fixed grid and the GPS every 5 s; the last two are served from the wait loop between cycles (`Sensors::service`, `background_due`). A reading carries the latest CO₂ and VOC values only while they are fresh (60 s and 90 s), so a stalled sensor shows as missing, not as a frozen value. Sensor state that must survive a restart (`lp8-channel<N>.txt`, `sgpc3-channel<N>.txt`) goes to `AcqConfig.state_dir`, the data folder.
- **Startup.** The app starts with no window; the dashboard webview is created on demand and destroyed when closed. Development instances (`--simulate` or `TOWER_BRIDGE_DATA_DIR`) skip the single-instance check, so they can run next to an installed copy.
- **Settings.** `settings.json` uses snake_case serde. On load, `snake_case_keys` converts files from older camelCase builds. `SETTINGS_VERSION` triggers upgrade steps; see `ChartStyles::upgrade` and `upgrade_ms_v5`. `ServiceState::apply_settings` diffs the old and new settings and applies only what changed.
- **Storage.** The `readings` table holds one row per timestamp and device in SI units; NULL means the sensor failed. History queries bucket rows to about 1000 points and return columns ready for uPlot. Buckets of whole hours (spans from about three weeks) read `readings_hourly` instead: per hour and channel the sum, count, min and max of each metric. `Store::insert`, `insert_many` and retention recompute the affected hours from `readings`, so anything else that writes `readings` must call `hourly::refresh` too.
- **Relay.** The state is `Option<bool>`: `true` is the T direction (green LED), `false` the F direction (red LED), and `None` is unknown (after power-up, since the relay is latching). On the wire unknown is `null`. A pulse switches, waits `duration_ms` measured from the moment the coil switched, then switches back. Clicking the running pulse again ends it early.

### Frontend

- Svelte 5 runes. State lives in `live.svelte.ts`, which loads a snapshot and then follows backend events.
- Alert limits show as a light red band over the values beyond them, drawn under the lines (uPlot `drawAxes` hook), with a faint line at the limit. They do not widen the y range, so a band appears only when the data comes near it. The user chose bands over the earlier dashed lines, which looked like the min and max lines (2026-10-18).
- Charts use uPlot through `TimeChart.svelte`. All charts on a page share a `ChartGroup`:
  - a plain drag pans and Shift+drag selects a range;
  - zoom needs Ctrl/⌘ with the scroll wheel, or a pinch, so plain scrolling still scrolls the page;
  - panning stays within the stored data.
- Chart line colours come from the 8-colour palette in `src/lib/palette.ts`, with separate light and dark values. These values passed the dataviz validator, so re-validate any change. The status colours (good, warning, critical) are reserved for status and are never used as series colours.
- The Dashboard layout (panel order) is kept in localStorage. The TOWER Bridge Module card is not a panel: it always comes last and cannot be dragged (the user, 2026-10-17). The GPS card comes before the NFC card (layout v3 moves it in saved orders).
- While the bridge works, the Dashboard shows only what is connected now: a module unplugged since has no line and no panel; its readings stay on the Data page, named "Climate Module R1 (disconnected)" from the module last seen there (`rememberSources`, kept in localStorage) (the user, 2026-10-18).
- The sidebar folds to an icon rail with "‹ Collapse" at its very bottom, a "›" there on the rail (the user, 2026-10-18; remembered in localStorage). It lists the connected modules A to Z, short names, no relay state; the Bridge Module card lists them by I²C channel, A to Z within a channel (the user, 2026-10-18).
- The GPS card's map is offline: `src/assets/map/world.json` (7.4 MB, about 1.5 MB compressed in the binary) is fetched only when a GPS card shows a position. It holds Natural Earth countries, lakes, rivers and populated places; `npm run map:build` rebuilds it from the sources named in `scripts/build-map.mjs`. Zoom levels match web maps (256 px tiles), so Natural Earth's `min_zoom` decides which places and rivers show.
- **The drag handle is the rightmost tool** in every card header (the user, 2026-10-17).
- **State never changes the font weight.** Active, current, selected or pressed items show it with colour, background, a border or an indicator. Bolder text is wider, so buttons grow and menus reflow (the user noticed it in the relay card and the manual's contents).

## Conventions

- **MQTT:** the app is only a client (rumqttc) of the user's broker; there is no built-in broker (rumqttd was removed on 2026-10-17: barely maintained, 157 crates, and it could not be stopped without an app restart). The client is replaced whenever its settings change. Publishing goes through `mqtt::Publisher`, which tracks its retained topics so it can clear them. The tests use a tiny broker in `mqtt.rs` (`TestBroker`), not a real one.
- **MQTT topics:** `{prefix}/{metric}/{channel}`, e.g. `tower/temperature/0`; the relay is `{prefix}/relay/{channel}` with `/set` and `/pulse` below it. The channel is always present (topics must not change when a second sensor appears), and topics never name hardware. Agreed with the user on 2026-10-17.
- **JSON:** keys are lowercase snake_case everywhere: REST, MQTT payloads, MCP, `settings.json`, Tauri IPC. Durations are integer milliseconds with an `_ms` suffix (`duration_ms`, `pulse_duration_ms`). Enum values are snake_case too. Names defined by a protocol keep their spelling (MCP `structuredContent`).
- **No em or en dashes.** This applies everywhere: code, comments, docs, UI texts and commit messages. Use a comma, a semicolon, a colon or parentheses, or split the sentence. A plain hyphen is fine. `src/repo.test.ts` fails on any long dash.
- **I²C:** write "I²C" in UI texts and docs, and `i2c` in identifiers.
- **Product names** keep their capitals: TOWER Bridge Module, TOWER Climate Module, TOWER Relay Module. The tags' full names are TOWER Humidity Tag, TOWER Temperature Tag, TOWER Lux Meter Tag, TOWER Barometer Tag and TOWER VOC-LP Tag (the user, 2026-10-18; on 2026-10-17 it was HARDWARIO). The NFC Tag is the TOWER NFC Tag. Dashboard cards are titled by what they do: "Relay" (translated), "NFC", "GPS"; the TOWER Bridge Module card keeps its name (the user, 2026-10-18). The other modules are the TOWER CO2 Module and TOWER GPS Module. The Bridge Module card and the ⓘ data sources show the full names (`productName` in `src/lib/source.ts`), the card with the revision set small; tight places (legends, tray, status, the sidebar) use the short ones.
- **Five languages:** English, Czech, Slovak, German, Polish. Every UI text goes into all five files in `src/lib/i18n/`; a test checks that the keys match. Tray and notification texts are in `src-tauri/src/i18n.rs`.
- **Rust:** edition 2024; `unwrap` and `expect` are denied by clippy outside tests. The core crate must not depend on Tauri.
- **Comments** explain why, not what, at the density of the surrounding code.
- **Settings migrations:** when a default or a format changes for existing users, bump `SETTINGS_VERSION`, add an upgrade step and test it with a file from the previous version.
- **The CLI:** English only. Data on stdout, messages on stderr as `error: ...` with an optional `hint: ...` saying the way out. Exit codes 0 ok, 1 error, 2 usage, 3 busy, 4 not found, defined in `main.rs` (`exit`) and listed in `docs/cli.md`; return them with `crate::failure(code, message, hint)`. `--json` prints the API's JSON (one object, or NDJSON with a `type` field for streams). Full product names in text output, short ones in the dashboard. Every new command gets an assert_cmd case in `crates/tower-bridge-cli/tests/cli.rs`, run against `--simulate` in a temporary `--data-dir`.

## The user manual

The manual is part of the product. It ships inside the app (the Manual tab) and is written in `docs/manual/<lang>.md` for all five languages.

- **Update it with every user-visible change, in all five languages, in the same change.** Use the exact UI labels from `src/lib/i18n/`.
- **Headings** are `## Title {#id}`. The ids are English, stable and the same in every language, because links and the table of contents use them.
- **Diagrams** are SVG files in `docs/manual/diagrams/`. They are inlined in the app, so they follow the theme.
  - Style them with the `mg-` classes and CSS variables with fallbacks. Presentation attributes cannot use `var()`; use `style`.
  - A `<text data-i18n="relay.pulseTrue">` element shows that UI label in the reader's language.
  - Keep other text language neutral, or number the callouts and explain them in the Markdown.
- **Screenshots** are `docs/manual/images/<lang>/<name>.webp`, each with a `.dark.webp` twin. Regenerate them with `npm run manual:screenshots`, never by hand.
- **Tests** in `src/lib/manual.test.ts` check that:
  - all manuals have the same sections and images;
  - the UI labels in `LABELS` appear verbatim in each manual;
  - every diagram label has a translation.

  When a label is renamed in the app, the test points at every manual to update.

## Rules for agents

- **Commit only when the user asks.** End commit messages with the attribution line requested in the session.
- **Respect the user's running copy.** The user runs TOWER Bridge from `/Applications` or from `./target/debug/tower-bridge` with their real data, and it holds the bridge and the data folder's lock.
  - Never kill or restart it unless the user asks.
  - Test with `--simulate` or `TOWER_BRIDGE_DATA_DIR=<temp dir>` (the CLI also takes `--data-dir`), and stop test instances by PID. A CLI command on the real data folder goes through the user's app (its API server) or refuses with exit code 3.
- **Only one program can open the FT260 at a time.** The CLI fails while the app holds the device. Ask before freeing it.
- **Keep TODO.md current.** List what is in progress, what comes next and what waits on hardware or a decision, so the next person can continue without a handover.
- **Record key learnings.** When you find a hardware quirk, a library pitfall, or a decision whose reason is not obvious from the code, add a dated bullet below in the same change.
- **Update the docs with the code:** README, `docs/*.md`, CHANGELOG and the manual.
- **Ask before deciding user-facing names and protocols:** MQTT topics, JSON fields, UI wording. The user has firm preferences: see the conventions above.

## Key learnings

Newest at the end. One bullet per learning, with the date and the reason.

- 2026-10-17. FT260 output and feature reports are padded to 64 bytes; macOS accepts this, as Windows requires. hidapi opens the device exclusively, so enmon, HARDWARIO Playground and this app exclude each other.
- 2026-10-17. Climate Module R1 is SHT20 (0x40) + TMP112 (0x48) + OPT3001 (0x44) + MPL3115A2 (0x60). R2 replaces SHT20 and TMP112 with SHT30 (0x45). Revision detection uses command writes (soft reset), not read probes.
- 2026-10-17. The Relay Module uses a TCA9534A at 0x38 to 0x3F (0x3B, 0x3F on the module) or a TCA9534 at 0x20 to 0x27 (0x23, 0x27). Probe both.
- 2026-10-17. The relay's left and right markings are swapped in hardware, so the states are named `true` and `false`, never left and right.
- 2026-10-17. Relay commands sent during a measurement used to wait up to a second. Serving them from the idle hook in `measure_with` fixed apparent "lost" MQTT commands.
- 2026-10-17. A pulse must be timed from the moment the coil switched, after the 300 ms recharge, not from when the command arrived; otherwise short pulses end early. There is a regression test.
- 2026-10-17. The sensor tags on channel 1 use the same chips as Climate Module R1, minus the TMP112: Humidity Tag SHT20 0x40, Lux Meter Tag OPT3001 0x44, Barometer Tag MPL3115A2 0x60. Use `tower-bridge-cli toolbox --channel 1 i2c-identify` to check.
- 2026-10-17. axum 0.8 writes path parameters as `/{param}`; the old `/:param` panics at startup.
- 2026-10-17. rust_xlsxwriter 0.100 returns a `Result` from `add_worksheet_with_constant_memory()`.
- 2026-10-17. TypeScript 7 lacks the compiler API that svelte-check uses; stay on TypeScript 6.0.x until svelte-check supports it.
- 2026-10-17. WebKit squeezes an empty flex item to zero width when its neighbour's text overflows. Status dots need `flex-shrink: 0`, or they turn into a thin bar.
- 2026-10-17. Cards must not use `overflow: hidden`, because it clips popovers such as the ⓘ source info. The red alert edge is an inset `box-shadow`.
- 2026-10-17. Simulated mode keeps its data and settings in `<data dir>/simulated/`, apart from real data.
- 2026-10-17. In SVG, `marker-end` marks only the last vertex of a path; draw every arrow as its own path.
- 2026-10-17. Chrome's DevTools `Page.captureScreenshot` encodes WebP, and `clip.scale: 2` renders sharp 2x images; no image tools are needed. Headless Chrome `--screenshot` with virtual time does not run smooth scrolling; use Playwright for anything interactive.
- 2026-10-17. Shell heredocs and some editing tools turn a backslash-u escape (such as the one for U+2013) into the character itself, which then trips the dash test. Write such escapes from Python, doubling the backslash.
- 2026-10-17. Climate Module R1 and the Humidity Tag share the SHT20 at 0x40; only the TMP112 tells them apart. A channel is named a Climate Module from its set of parts, and the SHT20 is the thermometer only when there is no TMP112 (or SHT30).
- 2026-10-17. Probing absent parts is harmless between measurements (soft reset writes and ID reads), so the periodic probe can run next to a working Climate Module and find a tag plugged in later.
- 2026-10-17. SQLite cannot change a primary key in place: adding the channel to `readings` meant copying into a new table (schema migration 2); the alert log only needed `ADD COLUMN`.
- 2026-10-17. A broker replays retained messages to a client whenever it subscribes (also after a prefix change), with `retain` set. Retained commands are therefore ignored; act only on live ones.
- 2026-10-17. The API server runs a current-thread tokio runtime: any backend call that waits for the bridge must go through `blocking()`, or every other request stalls.
- 2026-10-17. With `#[serde(default)]` on a struct, a missing `version` field reads as the current version and silently skips migrations. The loader inserts version 1 when the key is missing.
- 2026-10-17. Svelte 5 `$state` deep-proxies large objects such as a whole History. Use `$state.raw` for data that is replaced whole, and depend on a derived key rather than an array prop that is rebuilt on every render (uPlot otherwise redraws on every reading).
- 2026-10-17. macOS and Linux render the app with WebKit, Windows with Chromium. WebKit-only layout bugs (like the flex squeeze above) show up only in `npm run test:ui:webkit` (Playwright's WebKit) or on a Mac.
- 2026-10-17. uPlot leaves series with `show: false` out of drawing, bands, cursor points and the y auto-range, so hiding a chart line needs no data changes (`TimeChart` `lines[].hidden`).
- 2026-10-17. There is no prettier config or dependency: the TypeScript and CSS style is prettier with `--print-width 120`; Svelte files are not run through prettier, and neither is Markdown (hand-written, with compact `|---|---|` tables; prettier would pad every table and re-indent code blocks in lists). A plain `npx prettier --write` rewraps everything at 80 columns, and objects it splits stay split even after a later run at 120 (prettier keeps an object expanded when a newline follows its `{`).
- 2026-10-17. The FT260 reports no USB serial number (hidapi gives none; `tower-bridge-cli toolbox scan` shows `serial=-`), so the app names every bridge "ft260" and cannot tell two apart. Discovery opens the first usable one in enumeration order.
- 2026-10-17. Charts live on the Dashboard only; the History page duplicated them and became the Data page (table, custom range, export) at the user's request.
- 2026-10-17. Manual screenshots have no reserved size, so a lazily loaded one grows while the page scrolls to a chapter and pushes the chapter away. They load eagerly, and a jump waits for them. During a jump the contents keep the clicked chapter; letting the scroll spy run would sweep the highlight through every chapter passed. Never change the font weight of the active entry: wider text reflows the menu.
- 2026-10-17. The NFC Tag is an NXP NT3H2111 (NTAG I²C plus). HARDWARIO tags answer at 0x08 (as twr-sdk expects), not at the chip's factory default 0x55; the address is stored in block 0. A bare one-byte read does not find the chip, so `i2c-scan` misses it; `toolbox nfc-scan` finds it at any address. It wants a STOP between writing the memory address and reading a block (no repeated start). Never write block 0: a wrong byte moves the chip or locks it for good. The URL goes into user memory from block 1 as `03 len <URI record> FE`; each block needs about 5 ms of EEPROM programming. Verified on a real tag: read, write and a byte-exact restore.
- 2026-10-17. The Temperature Tag's TMP112 sits at 0x48 like the Climate Module R1's thermometer; on one channel they collide (both answer every read). Its alternative address 0x49 needs R4 fitted and R5 removed.
- 2026-10-17. The TOWER CO2 Module's LP8 datasheet ("LP8 sensor module specifications and integration guideline" v1.02) is published by Senseair. Report Filt_CO2_Pres (0xAA), the value the accuracy is specified for; write back the 23-byte sensor state every time, or the filters and ABC start over; keep 16 s between measurements; ask for ABC (control 0x70) every 8 days. The module's TCA9534A is switched by its direction register with the output latch at 0 (output = line low = on); the SC16IS740 crystal is 13.56 MHz, so the divisor for 9600 baud is 88. twr-sdk recharges the capacitor for 5 s before each measurement because it targets batteries; on USB the capacitor stays on charge.
- 2026-10-17. The SGPC3 on the VOC-LP Tag must be switched to ultra-low power (`set_power_mode 0`) before `init_preheat` when it is read every 30 s; the default mode's algorithm expects 2 s. It reads 0 ppb while preheating, which is not a value.
- 2026-10-17. The GPS Module's SAM-M8Q sends NMEA on I²C by default; over the FT260 (about 25 ms per transfer) reading that stream is too slow, so the app switches the port to UBX output and polls UBX-NAV-PVT. Its expander at 0x21 is generic, so a GPS Module counts only once the receiver at 0x42 answers.
- 2026-10-17. TCA9534 expanders and the NT3H2111 do not acknowledge a bare one-byte read, so `i2c-scan` falls back to writing register pointer 0. `i2c-read` and `i2c-write` in the CLI help with new modules.
- 2026-10-17. An inline grid (`display: inline-grid`) with `1fr` button columns sizes every such column to the widest button, which gives equal buttons with aligned edges without fixed widths (the relay card). Subgrid and container queries would be neater but need Safari 16, newer than the WebKit of macOS 11, the app's minimum.
- 2026-10-17. Natural Earth's populated places have English, German and Polish names but no Czech or Slovak ones; the map builder takes those from Wikidata labels by the places' `WIKIDATAID` (CC0). The NE `NAME` is the English exonym ("Prague"), so the builder stores a language's name only where it differs.
- 2026-10-17. Vitest 5 caches transformed modules in `node_modules/.vitest-cache`, including the result of `import.meta.glob`; a new file that matches a glob (such as a new manual screenshot) stays invisible to the tests until that folder is deleted.
- 2026-10-18. A background job in the acquisition wait loop must move its next time on even when it cannot run (its channel cannot be selected); a job that stays due is served again at once, forever. Unplugging the bridge with a VOC-LP Tag or GPS Module attached spun the thread at 100 % CPU and it never reconnected. `Sgpc3::missed`, `GpsModule::missed` and `Flow::Lost` fix it, with a regression test that unplugs a simulated bus.
- 2026-10-18. An empty measurement cycle counts as failed only when parts reported errors: a CO2 Module charging its capacitor (first minute) or a VOC-LP Tag preheating (five minutes) has nothing to report yet, and counting those cycles made a bridge with only such modules reconnect forever.
- 2026-10-18. rumqttc 0.25: dropping the link must not stop its thread before the queued messages and the disconnect went out (only one request is sent per poll); wait for `Outgoing::Disconnect`. Its `TlsConfiguration::default()` panics when one system certificate cannot be loaded, so the app builds the rustls config itself.
- 2026-10-18. rusqlite's `execute_batch` steps a statement once, and `PRAGMA incremental_vacuum` frees one page per step: step it to the end, or the file never shrinks. `SELECT MIN(ts), MAX(ts)` scans the table; two subqueries use the index.
- 2026-10-18. axum 0.8's `serve` has no header or idle timeout; the API server wraps the listener (`axum::serve::Listener`) to cap connections and close idle ones. Hyper keeps reading while a handler runs, so the idle timeout must be longer than any answer takes (30 s).
- 2026-10-18. An error returned from Tauri's `setup` ends in a panic that cannot unwind on macOS, with no message and no log. `setup` shows a native message box and exits instead.
- 2026-10-18. Queued relay commands are coalesced, the last one wins: each switch takes 300 ms or more, and a burst (a looping automation, many API calls) used to keep clicking long after the callers gave up.
- 2026-10-18. On the GPS Module (schematic rev 1.2) only P0 (receiver supply), P3 and P5 (LEDs) and P7 (backup supply) of the TCA9534 are connected. Until the receiver answers, only those are outputs, since 0x21 may be another device.
- 2026-10-18. A popover sized `width: max-content` with a `max-width` lets unbreakable text spill out of its box. Give long text places to wrap (the ⓘ data path wraps between its "→" steps), and open popovers toward the side with room, since the scrolling `<main>` clips anything that sticks out (`ColorPicker`, `SourceInfo`).
- 2026-10-18. Every duration in JSON is integer milliseconds with `_ms` (`cooldown_ms`, `sample_interval_ms`, MCP `repeat_after_ms`). Settings version 6 converts the older `_s` keys before deserializing, since serde would otherwise drop them silently and fall back to the defaults.
- 2026-10-18. Third-party notices: `scripts/notices.mjs` walks `cargo metadata` (normal dependencies only, filtered to the four shipped targets) and the module ids of a `vite build` (only what is in the bundle). The Unicode license repeats the MIT wording and some crates ship one file with both MIT and Apache texts, so a text can match several licenses; a file of the chosen license alone wins. A dual-licensed crate is listed under MIT where it allows it. The file quotes license texts verbatim and is exempt from the dash test.
- 2026-10-18. GitHub Actions on Node 20 are past end of life: checkout, setup-node, upload-artifact and download-artifact need v7 or later (download v8), action-gh-release v3. Actions are pinned to commit SHAs with the version in a comment; `dtolnay/rust-toolchain` pinned to a commit needs `toolchain: stable` spelled out. Look SHAs up with `git ls-remote https://github.com/<repo> 'refs/tags/<tag>^{}'`.
- 2026-10-18. The hourly summary is recomputed from the hour's readings on every insert (at most 1800 rows per channel at the 2 s minimum interval, under a millisecond) instead of being updated incrementally: `INSERT OR REPLACE` does not fire delete triggers, and a deleted minimum cannot be undone incrementally. Batches refresh each hour once at the end; a test that inserts thousands of rows one by one is slow for that reason. The 30-day chart query fell to about 1 ms per channel.
- 2026-10-18. A scroll box with a sticky header or column keeps its scrollbar clear of them with `::-webkit-scrollbar-track { margin-top: ... }` (or `margin-left`), sized from the sticky cell (the Data table). It works in WebKit and Chromium because `app.css` styles `::-webkit-scrollbar`; setting the standard `scrollbar-width` or `scrollbar-color` anywhere would make Chromium ignore all of it. Headless Chrome hides scrollbars: check with Playwright's `ignoreDefaultArgs: ["--hide-scrollbars"]`.
- 2026-10-18. The instance lock is `std::fs::File::try_lock` (flock on Unix, LockFileEx on Windows): it conflicts per open file, also within one process, and dies with the process. Windows does not let another process read a locked file, so the holder's name in the lock file is shown only on Unix. Probing the lock (`InstanceLock::holder`) takes it for an instant, so `acquire` retries for a moment before calling a folder taken. A test that starts `run` in the background waits for its lock before running other commands, which would otherwise open the bridge in a session of their own.
- 2026-10-18. The CLI dashboard sends tracing into a ring buffer shown in its log pane (`tui::LogBuffer`, a `MakeWriter`): anything on stderr tears a raw-mode screen. The bridge is opened before `ratatui::try_init`, so a failure prints as a normal error; Ctrl+C arrives as a key in raw mode, SIGTERM through `ctrlc`.
- 2026-10-18. A GPS Module is announced with the receiver's first report, about 5 s after detection, because its expander's address is generic and only the receiver confirms the module. `status` in a short session therefore may not show it; `gps` waits up to 12 s for it.
- 2026-10-18. `Terminal::new(TestBackend)` cannot fail (its error is `Infallible`): write `let Ok(terminal) = ...;` without `else`, or clippy flags an unreachable branch.
- 2026-10-18. Long-span history reads the hourly summary only for the whole hours inside the range and the readings for the parts of the first and last hour; summarizing whole edge hours counted readings outside the range (custom ranges, MCP statistics, time zones off the hour). `Store::open` also summarizes the hours an older build recorded without the table, which otherwise stayed gaps for good.
- 2026-10-18. `npm run notices` sorts by code points: `localeCompare` follows the locale, and Czech sorts "ch" after "h", so the file differed between machines and failed the CI check.
