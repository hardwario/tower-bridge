# TODO

Work in progress and open items, kept current so anyone (person or agent) can pick up where the last session stopped. Update this file whenever you start, finish or defer a task. Move finished items to CHANGELOG.md, not here. Architecture and rules are in AGENTS.md.

## In progress

Nothing.

The work up to 2026-10-17 is committed on the branch `feature/channels-manual-mqtt` (not merged into `main` yet).

## Done, to be confirmed by the user

- **MQTT is a client of the user's broker only** (2026-10-17). The built-in broker (rumqttd and its 157 crates) is gone.
  - Settings: `mqtt.host`, `port`, `tls`, `username`, `password`, `client_id`, `topic_prefix`, `allow_control`. Settings version 4.
  - A settings file that used the built-in broker turns MQTT off (logged); one set up for an external broker keeps it.
  - It reconnects by itself and republishes on connect; `tower/status` is `online`, with `offline` as the last will.
  - Tested against a real Mosquitto before the removal (retained values, a relay command, the last will, republishing after a broker restart); since then by the unit tests with a tiny test broker.
  - TLS was not tested against a real broker.
- **Hide a chart line from the legend** (2026-10-17). Click a line's name in a panel or History legend; the row turns grey. Saved per metric (`charts.<metric>.hidden_channels`), so the dashboard and History agree. One line always stays.
- **Module list in the Bridge Module card** (2026-10-17). One module per line after its channel ("I²C channel 0: TOWER Climate Module R1"), by full product name with the revision set small and muted, the Relay Module included. Tags are "HARDWARIO Humidity Tag" and so on (the user's naming). Elsewhere (status pill, chart legends, tray, notifications) the short names stay. The user asked for "I2C bus 0: ..."; "channel" was kept to match the rest of the app, pending the user's word.
- **Data page instead of History** (2026-10-17). Table, the Dashboard's time spans (`SPANS` in `src/lib/time.ts`, shared) or custom dates (on their own row, so the row above never moves), CSV and XLSX export; no charts. Tab key, manual id and screenshot are `data`.
- **Manual contents fixed** (2026-10-17). No bold active entry (it reflowed the menu), no highlight sweep during a jump, and jumps land on the chapter (screenshots load eagerly).
- **Several TOWER Bridge Modules: a warning** (2026-10-17, the user chose the warning over multi-bridge support). The acquisition thread counts the bridges when one opens, every minute and on Rescan (`Bridges::count`, `DeviceInfo.bridges_connected`); the card and the tray status warn while more than one is plugged in. Tested with a fake opener only; no second bridge was available.
- **TOWER NFC Tag** (2026-10-17, asked by the user). NT3H2111 at 0x55 on either channel, found with the other parts (every minute, so a URL written by a phone shows up). Dashboard card: the URL on the tag, a field and Write (NDEF URI record, prefix codes, read back to verify; block 0 is never written). CLI: `tower-bridge-cli nfc [URL]` and `nfc --scan` (every address). HARDWARIO tags sit at 0x08 (the chip's default 0x55 is tried too). Verified on the user's tag (channel 0, 0x08, holding `https://www.bigclown.com/`): read, write of a test URL, and a byte-exact restore of the original. The card's Write button itself was tested in the browser preview, not yet clicked in the real app. Not in REST, MCP or MQTT yet.
- **Decimal separator setting** (2026-10-17). `decimal_separator`: `auto` (the language), `point` or `comma`; Settings, General. Applies to the UI (`fmtNumber`, `fmtInput`), the tray and notifications (`format::Decimal`). A digit group separator that would look like the chosen decimal one becomes a narrow space. Exports keep their format (XLSX numbers, CSV with a point).
- **Sidebar device list** (2026-10-17): one module per line in the Bridge Module card's order, without the TOWER/HARDWARIO prefix (the user: too long there); the status pill only while searching, unplugged or without sensors.
- **New Dashboard panels land at their default place** (2026-10-17): a saved panel order gets a new card (the TOWER NFC Tag) before its default successor, here the Bridge Module card, instead of at the end.
- **Drag handle rightmost** (2026-10-17) in the metric panels too; the manual's panel diagram follows.
- **Interval in the Dashboard header** (2026-10-17), right of "Last update" (the user moved it there from the Bridge Module card). A dropdown: 5 s, 10 s, 20 s, 1 min, 3 min (10 s kept as the old default; a value set through the API shows as an extra entry). Removed from Settings. Default 20 s; settings version 5 moves the old default 10 s to 20 s.
- **Relay buttons keep their width** (2026-10-17): the current state is marked by border, background and the lit LED, not bold text.
- **Data table sorting** (2026-10-17). Click a column head; again to reverse. Rows without a value in that column go last.
- **No serial number in the UI** (2026-10-17): the FT260 has none (the app shows "ft260" for every bridge).
- **WebKit UI smoke test** (2026-10-17): `npm run test:ui:webkit`, also in CI. It checks layout too (overflow and squeezed dots); reintroducing the old WebKit dot bug makes it fail.

- **Alternative tag addresses** (2026-10-17): Temperature Tag TMP112 at 0x49 besides 0x48, Lux Meter Tag OPT3001 at 0x45 besides 0x44. Tested with the mock bus only; no tag at an alternative address was available.

- **Sensors on both I²C channels** (2026-10-17).
  - Dashboard: one panel per measurement, a line per channel.
  - MQTT: `tower/{metric}/{channel}` and `tower/relay/{channel}`.
  - Checked on the real hardware: Climate Module R1 on channel 0; Humidity, Lux Meter and Barometer Tags on channel 1; Relay Module on channel 0.
  - The real database migrated to schema 2. A backup is in `<data dir>/backup-before-channels-2026-10-17/`; delete it once the user is happy.

- **TOWER CO2 Module, VOC-LP Tag, GPS Module** (2026-10-17, the user chose the names and topics: `co2` in ppm, `voc` in ppb, `tower/gps/<ch>/latitude` and so on, GPS without history).
  - Checked on the real hardware with the CLI: LP8 measurement about 0.7 s, about 870 ppm filtered and pressure compensated, healthy VCAP, state file kept; SGPC3 0 ppb while preheating, then about 74 ppb; GPS configured and polled, no fix indoors.
  - Checked in an isolated app instance: all three found on channel 0 next to the Temperature Tag, the GPS Module and the NFC Tag; CO₂ stored at about 80 s.
  - Seen in the real app (the user's database, 2026-10-17 21:50 to 22:25): CO₂ 864 to 919 ppm; VOC 0 to 145 ppb, but 0 in more than half of the readings. Zero is what the SGPC3 reports for clean air, and its algorithm learns for hours; the baseline is stored only after an hour of running, so the frequent restarts during development kept resetting it. Check again after a day of uninterrupted running.
  - Still to see: a GPS fix (needs sky view), an ABC run (8 days).
  - No mock-based tests yet for the CO2, VOC and GPS parts of the acquisition loop (a `SimLp8`, `SimSgpc3` and `SimGps` in `bus/mock.rs` would make them possible).

- **Offline map in the GPS card** (2026-10-17, asked by the user, budget 3 MB compressed). Natural Earth 1:10m countries, lakes, rivers and places (`src/assets/map/world.json`, 7.4 MB, 1.5 MB brotli), place names in all five languages; drag, zoom buttons, Ctrl/⌘+scroll and pinch, back to the position. Tested in the browser preview (Chrome, WebKit smoke test); not yet with a real fix.
- **Relay card as a grid** (2026-10-17, the user's layout): "Manual control" and "Pulse control" rows, equal buttons, the duration after the pulse buttons (its own line below 1000 px window width). The row names are the user's wording, translated.
- **TOWER Bridge Module card always last** (2026-10-17, asked by the user): no longer a draggable panel; saved layouts drop it.

## Open questions from the review

- The acquisition loop's reopen after five failed cycles has no test, because the loop waits in real time. A clock-driven wait would make it testable.
- The Identify blink (about 0.75 s) still delays relay commands.


## Deferred (the user decided to postpone)

- Let the user name the channels (for example "Indoors" or "Outdoors") and show those names in the panels, legends, notifications and MCP output instead of the module names. Deferred on 2026-10-17.

- A built-in MQTT broker again, for users without one (removed on 2026-10-17: rumqttd was barely maintained and could not be stopped without a restart). Options when it comes back: a maintained embeddable broker, or bundling Mosquitto. Deferred on 2026-10-17.

## Waiting for hardware

- Windows and Linux builds have not been tested on real hardware.
- Climate Module R2 (SHT30) has not been tested on real hardware.
