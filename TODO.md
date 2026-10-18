# TODO

Work in progress and open items, kept current so anyone (person or agent) can pick up where the last session stopped. Update this file whenever you start, finish or defer a task. Move finished items to CHANGELOG.md, not here. Architecture and rules are in AGENTS.md.

## In progress

Nothing.

The work up to 2026-10-17 is committed on the branch `feature/channels-manual-mqtt` (not merged into `main` yet). The overnight review of 2026-10-18 (below) is not committed.

## Overnight review, 2026-10-18 (uncommitted, to be confirmed by the user)

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
- **Docs and translations:** README, api.md, mqtt.md, hardware.md, the manual (dashboard cards, translations in all languages) and the CHANGELOG (rewritten as one list for 0.1.0); about 60 translation fixes from a native-level review (Czech, Slovak, German, Polish).
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

- Rename the JSON durations to integer milliseconds: `cooldown_ms` (alert rules), `sample_interval_ms` (status, MCP), MCP `repeat_after_ms`; settings migration.
- Third-party notices generated for every crate, npm package, font, icon set and the map data, shown in the Manual's About chapter.
- An hourly summary table for long chart spans (schema migration).
- Alert limits on charts as a faint band outside the range instead of dashed lines.
- Release workflow: actions pinned to commit SHAs, and a Windows check that the exe needs no VCRUNTIME DLL.
- Kept as they are: the brand red for switched-on toggles; the word "I²C channel" (not "bus").

## Open questions from the review

- The acquisition loop's reopen after five failed cycles has no test, because the loop waits in real time. A clock-driven wait would make it testable.
- The Identify blink (about 0.75 s) still delays relay commands. Low priority: fixing it means timing the LED toggles in the wait loop like a pulse, for a rare case.


## Deferred (the user decided to postpone)

- Let the user name the channels (for example "Indoors" or "Outdoors") and show those names in the panels, legends, notifications and MCP output instead of the module names. Deferred on 2026-10-17.

- A built-in MQTT broker again, for users without one (removed on 2026-10-17: rumqttd was barely maintained and could not be stopped without a restart). Options when it comes back: a maintained embeddable broker, or bundling Mosquitto. Deferred on 2026-10-17.

## Before the first release

- Push the repository and get CI green: the workflows have never run.
- Panels can be reordered with the pointer only; arrow-key reordering would make it keyboard accessible.

## Waiting for hardware

- The tray menu's metric lines are added and removed at run time (`Menu::insert`, `remove`); check them on a macOS menu bar, a Windows tray and a Linux AppIndicator.

- Windows and Linux builds have not been tested on real hardware.
- Climate Module R2 (SHT30) has not been tested on real hardware.
