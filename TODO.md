# TODO

Work in progress and open items, kept current so anyone (person or agent) can pick up where the last session stopped. Update this file whenever you start, finish or defer a task. Move finished items to CHANGELOG.md, not here. Architecture and rules are in AGENTS.md.

## In progress

**tower-bridge-cli as the headless sibling of the desktop app** (asked for on 2026-10-18). Decided with the user:

- Scope: everyday commands plus `run`, a service without a window (history, alerts, MQTT, REST, MCP), and a terminal dashboard (ratatui), the default of a bare `tower-bridge-cli` in a terminal.
- Name stays `tower-bridge-cli`; settings, database and data folder are shared with the app; the bring-up commands move under `toolbox`.
- Commands: `dashboard`, `run`, `status`, `watch`, `relay` (`true`, `false`, `pulse [--ms]`), `gps`, `nfc` (`nfc write <url>`), `identify`, `rescan`, `export` (`--from --to` or `--last 7d`), `alerts`, `config` (`show`, `get`, `set`), `mcp`, `toolbox`. Global `--data-dir`, `--simulate`, `--json`, `--color`. Exit codes 0 ok, 1 error, 2 usage, 3 bridge busy, 4 not found.
- The REST API gains `GET`/`PUT /api/v1/nfc` and `nfc` and `mqtt` in `/api/v1/status`; MCP shows the NFC Tag in `get_device_status`.

Steps:

1. Done: `crates/tower-bridge-service`, the app's hub, settings, paths, state, control and API backend, shared by the app and the CLI; an instance lock per data folder.
2. Done: `Mode::Session` for short CLI commands (nothing recorded, published or served; settings only read).
3. Done: NFC and MQTT in the API (`GET`/`PUT /api/v1/nfc`, `nfc` and `mqtt` in the status, OpenAPI, api.md). MCP shows the NFC Tag in `get_device_status`, next to the relay and GPS, instead of a separate `read_nfc` tool.
4. Done: the CLI in modules (`access.rs` decides between a running instance and the bridge; `remote.rs` is the API client); text or NDJSON output; exit codes. Checked against a simulated desktop app holding the bridge.
5. Done: the terminal dashboard (`tui/`), checked in a pseudo-terminal.
6. Done: tests (unit, TestBackend renders, assert_cmd contract against the simulated bridge); docs/cli.md (with systemd and launchd examples), README, CHANGELOG, AGENTS, a "Command line" chapter and a troubleshooting entry in all five manuals; release archives of the CLI in release.yml (Linux x86_64 and aarch64 through `cross` with a glibc 2.28 floor, macOS universal, Windows with the static CRT check), a `workflow_dispatch` dry run, and the third-party notices covering the CLI.

Committed (3636cdc). A second review the same day found about 40 issues, fixed after it (see the 2026-10-18 learnings in AGENTS.md). Still open:

- The CLI's Linux builds in the `cross` images (Ubuntu 16.04) work: the Cross workflow built x86_64 and aarch64 (and macOS Intel) on GitHub on 2026-10-19, aws-lc included. The release workflow's own CLI jobs and its dry run have not run yet.
- A 32-bit Raspberry Pi build (`armv7-unknown-linux-gnueabihf`) is not in the matrix yet.
- Tried on real hardware on 2026-10-19 (macOS; Temperature Tag, CO2 Module, VOC-LP Tag, Relay Module, GPS Module, NFC Tag on channel 0; Humidity, Lux Meter and Barometer Tags on channel 1): every toolbox tool, the session commands, `run` with REST, MCP (HTTP and stdio) and MQTT against a local mosquitto, remote commands through it, the dashboard in a pseudo-terminal, Ctrl+C and SIGTERM, and a pulse switched back when `run` stops. Fixed after it: the GPS Module shows as soon as the receiver acknowledges its setup (its first report took up to 2 minutes), `toolbox voc` reports the preheat (up to 5 minutes; 90 s on this tag) and counts only values, `toolbox gps` prints text, `toolbox i2c-identify` names the modules' expanders, UART bridge, SGPC3 and receiver, `config get <group>` masks secrets, no "gone" log lines when the service stops. Not covered: a GPS fix (indoors), unplugging the bridge while the CLI runs.
- Formatting an unformatted NFC Tag (asked on 2026-10-18): not supported; the app, the CLI and the API (409 `not_formatted`) point to a phone app such as NXP TagWriter. Doing it means writing block 0, which also holds the I²C address and the static lock bytes. Per the NT3H2111 datasheet (to verify): byte 0 written sets the address (as `address << 1`) while a read returns the manufacturer ID 0x04, so a plain read-modify-write moves the chip; the lock bytes must be written back as read (0); the capability container is bytes 12 to 15 (`E1 10 6D 00` on the 1k part), then an empty NDEF TLV `03 00 FE` in block 1. Needs a spare real tag to verify.
- The release dry run (`workflow_dispatch`, Actions, Release, Run workflow) has not been tried yet; `release.yml` is on `main` since 2026-10-19.
- The Tauri capability grants `core:default`; the dashboard needs only `core:event:allow-listen` and `allow-unlisten` (and app commands). Narrow it once it can be checked on a running dashboard.
- Retained MQTT topics of a sensor that left while the app was closed stay on the broker (the published set lives in memory). Clear them by learning the retained topics at connect, or by storing the set.
- XLSX exports write local wall-clock times only; the repeated hour at the end of daylight saving time cannot be told apart. A UTC or Unix-ms column would fix it (a change of the export's columns: ask first).
- The Linux AppImage bundles LGPL libraries (WebKitGTK, GTK, GLib); the notices name them and point to the Ubuntu sources. Have someone check that this is enough for LGPL.

Published on 2026-10-19 as the public repository https://github.com/hardwario/tower-bridge; `main` is the only branch.

## Overnight review, 2026-10-18 (committed)

Five reviews (Rust core, security, docs and translations, UI, release) found about 80 issues. Fixed:

- **Acquisition:** unplugging with a VOC-LP Tag or GPS Module attached no longer spins the thread at 100 % CPU without reconnecting; a bridge with only a CO2 Module or VOC-LP Tag no longer reconnects forever during warm-up; a VOC-LP Tag dropped between cycles leaves the sources; CO2 and VOC modules survive Rescan and re-detection (no new 60 s charge, no lost SGPC3 baseline); a stale barometer no longer compensates the LP8; the LP8 state file is deleted after a fault; another device at 0x21 no longer hides a GPS Module on the other channel, and only the four connected expander pins are driven until the receiver answers. Regression tests for each, with simulated LP8, SGPC3 and u-blox devices in `bus/mock.rs`.
- **Relay:** a burst of queued commands switches once (the last wins) and answers every caller.
- **MQTT:** a prefix change or stop sends every clear and `offline` before disconnecting; TLS with an unreadable system certificate no longer kills the hub thread (which now also restarts after a panic); topics and alerts of a sensor that left a channel are cleared per metric; the GPS topics of an old channel too.
- **Alerts:** editing a rule no longer fires an active alert again.
- **API:** at most 64 connections, idle ones closed after 30 s; tokenless access only from http(s) pages on localhost (no more `tauri://` or `*.localhost`); MCP batches of at most 32 calls, with header checks; the CSV export is streamed; MCP texts mention CO₂, VOC and GPS, and no longer the meaningless serial number; the OpenAPI document lists every route and error.
- **Storage and settings:** retention really shrinks the database; the data range query uses the index; a damaged database is moved aside and replaced; settings are flushed to disk and readable by the user only; the `--mcp-stdio` bridge reads the settings without ever moving the file; development instances never take over Open at Login.
- **Startup:** a fatal error shows a message box instead of crashing silently.
- **NFC:** an interrupted write leaves an empty tag, never a mix of two URLs.
- **Tray:** the menu lists a line per measured metric (CO₂ and VOC included), none without a bridge. Not yet seen on a real tray.
- **Notifications:** the channel in titles is translated.
- **UI:** the colour picker no longer gets clipped; legends show the unit once, so names fit; the sidebar scrolls in small windows; the manual is one column below 1000 px; the big readings keep their decimal point; light-theme contrast; the chart tooltip only in the hovered chart, with labelled min and max; the GPS card keeps its size while searching; the Data table keeps its time column; aligned alert fields; recent alerts wrap cleanly; neutral hover; one Tab stop per segmented control; the drag handle is no dead Tab stop; selects drawn the same everywhere; the Bridge card lost its empty gap; the Linux permission banner gives a command that works with the AppImage; the NFC card confirms a write; the manual's image viewer has a close button; a missing value is a muted placeholder.
- **Docs and translations:** README, api.md, mqtt.md, hardware.md, the manual (dashboard cards, translations in all languages) and the CHANGELOG (rewritten as one list for the first release); about 60 translation fixes from a native-level review (Czech, Slovak, German, Polish).
- **Release:** CI tests the whole workspace on all three OSes with read-only permissions; the release runs the tests first and creates the draft once; `check-version` covers the workspace and the CHANGELOG.
- **Checked in an isolated `--simulate` instance of the built app** (the simulated bridge now has every module): CO₂, VOC and a GPS fix over REST and Prometheus, no warnings in the log, and 20 concurrent relay requests all answered at once.

A final independent review of the night's diff found one regression, fixed with a test: a stuck I²C switch counted as "no value yet" and was never recovered. Also fixed from it: MQTT reconnects after a hub restart; switching a rule off ends its alert at once; no tray lines without a bridge; a non-blocking fatal message on Linux; a CSV export failing midway is reported; the damaged database keeps its write-ahead log; the Rescan result sits beside the buttons; the interval dropdown text is centred; narrow manuals mark the chapter being read.

### Asked for on the morning of 2026-10-18 (done)

- The map no longer draws bands across the north when zoomed out: outlines crossing the 180th meridian (Russia, Fiji, Antarctica) are continued across it.
- Card titles "NFC" and "Relay" (translated); the GPS card before the NFC card, also in saved layouts; tags are named "TOWER … Tag"; the ⓘ data sources show full product names (TOWER Barometer Tag), and the GPS card has one too.
- The sidebar lists the modules A to Z, without the relay state.
- "API server port" instead of "Port".
- The Dashboard shows only connected modules; stored lines of unplugged ones were "Kanál 0" and are now on the Data page as "… (disconnected)".
- The relay card says "State:" before its dot; the Bridge Module card sorts the modules A to Z within each channel; the move and zoom hint sits right above the first chart; the ⓘ reads "I²C (chip FT260) → channel 0 → address 0x48 (chip TMP112)" in normal type and opens to the side with room.
- The sidebar collapses to an icon rail ("‹ Collapse" at its bottom); the Data table fills the window; the manual lists the sensor tags as a sublist; the map stops at the poles.
- The tray shows no "Updated" line while readings are fresh, only a status line when something is wrong or the readings are overdue (two intervals and a minute). A connected sensor without a value yet shows a spinner instead of "-".

### Decided by the user (2026-10-18), to do next

- Done: the JSON durations are integer milliseconds, `cooldown_ms` (alert rules), `sample_interval_ms` (settings, status, MCP), MCP `repeat_after_ms`; settings version 6 converts older files.
- Done: alert limits on charts are a light red band over the values beyond the limit, with a faint line at the limit, instead of dashed lines.
- Done: third-party notices (`THIRD-PARTY-NOTICES.md`, `npm run notices`) for every crate, bundled npm package, font, icon set and map source, opened from the Manual's About chapter ("Third-party notices"); CI checks that they are current.
- Done: workflow actions pinned to commit SHAs, moved to their Node 24 majors (Node 20 is past end of life); the release checks that the Windows exe imports no C runtime DLL. Not yet run on GitHub.
- Done: an hourly summary table (`readings_hourly`, schema migration 4) for chart buckets of an hour or more; checked against a copy of the real database (migration 9 ms, identical buckets).
- Kept as they are: the brand red for switched-on toggles; the word "I²C channel" (not "bus").

## Open questions from the review

- The acquisition loop's reopen after five failed cycles has no test, because the loop waits in real time. A clock-driven wait would make it testable.
- The Identify blink (about 0.75 s) still delays relay commands. Low priority: fixing it means timing the LED toggles in the wait loop like a pulse, for a rare case.

## Deferred (the user decided to postpone)

- More visuals for the README: photos of a real setup (a bridge with modules on both channels), of the VOC-LP Tag and the NFC Tag (the docs site has none), and a screenshot of the tray menu. Not wanted for now (the user, 2026-10-20).

- Let the user name the channels (for example "Indoors" or "Outdoors") and show those names in the panels, legends, notifications and MCP output instead of the module names. Deferred on 2026-10-17.

- A built-in MQTT broker again, for users without one (removed on 2026-10-17: rumqttd was barely maintained and could not be stopped without a restart). Options when it comes back: a maintained embeddable broker, or bundling Mosquitto. Deferred on 2026-10-17.

## 1.0.1

- Published on 2026-10-19 (https://github.com/hardwario/tower-bridge/releases/tag/v1.0.1), the first release the workflow built without hand repairs. Tested by the user from the draft: the notification permission prompt at the first start, **Send test** shows a banner and a click opens the dashboard, Open at Login appears as TOWER Bridge under Open at Login. Not yet seen on macOS 11 or 12 (launch agent kept there) or with notifications turned off on a real install.

## Before the first release

- CI is green on GitHub since 2026-10-19 (the first run failed only on a constant that is dead code on Windows).
- v1.0.0 was published on 2026-10-19 (https://github.com/hardwario/tower-bridge/releases/tag/v1.0.0) with all 15 files: the Windows and macOS signing worked on GitHub (the Windows job after the OIDC subject fix, see AGENTS.md). Two files were repaired by hand in the draft: the Linux CLI archives (left out by an artifact-name bug) were uploaded from the run's artifacts, and the macOS CLI archive was repacked with the executable bit; the workflow is fixed for the next release. Everything the signing needs exists: the GitHub environment `signing` (tags `v*`, no required reviewer) with its variables and secrets, and on Azure the app `github-codesign` (client ID 33ad3a9c-0938-406a-816b-4937325ac58c) with the GitHub OIDC trust (subject `repo:hardwario@28539912/tower-bridge@1425884115:environment:signing`) and the role Artifact Signing Certificate Profile Signer on the `hardwario` profile only.
- macOS signing and notarization (the `sign-macos` job, written on 2026-10-19, see docs/release.md): the Developer ID Application certificate "HARDWARIO a.s. (C32QFQ68V9)" was created on 2026-10-19 by the Account Holder from a key generated here (the API answers 403 to anyone else) and is **valid until 2027-11-18: renew it before then** (a new certificate request, uploaded by the Account Holder, then a new `.p12` in the secrets). The 2021 certificates (HARDWARIO s.r.o.) expired on 2026-08-30. The API key W6NY8BG8Q4 (shared with the mobile app builds on Codemagic) notarizes. The `signing` environment holds the secrets `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD` and `APPLE_API_KEY_P8` and the variables `APPLE_API_KEY` and `APPLE_API_ISSUER` (since 2026-10-19). The job ran on GitHub for v1.0.0: app, dmg and CLI accepted by Apple, `spctl` reports Notarized Developer ID.
- macOS with the theme set to System and the Mac in dark mode: the dashboard's title bar stays light while the page is dark (seen 2026-10-19 on the debug build with a fresh data folder; tao leaves the app appearance alone when no theme is given, and the binary declares SDK 27). Light and Dark themes are fine.
- Panels can be reordered with the pointer only; arrow-key reordering would make it keyboard accessible.

## Waiting for hardware

- The tray menu's metric lines are added and removed at run time (`Menu::insert`, `remove`); check them on a macOS menu bar, a Windows tray and a Linux AppIndicator.

- Windows and Linux builds have not been tested on real hardware.
- Climate Module R2 (SHT30) has not been tested on real hardware.
