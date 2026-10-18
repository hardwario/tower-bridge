# tower-bridge-cli

`tower-bridge-cli` is the headless sibling of the TOWER Bridge desktop app. It
does everything the app does without a window:

- shows the values in a terminal dashboard;
- prints them for scripts, as text or JSON;
- switches the TOWER Relay Module and writes the TOWER NFC Tag;
- exports the history;
- runs as a service on a computer without a desktop, such as a Raspberry Pi
  next to the bridge. The service records the history, checks the alert
  limits, publishes to MQTT and serves the REST API, Prometheus metrics and
  MCP.

It shares the app's data folder: the same settings, the same history.

## Install

Release archives (`tower-bridge-cli-<version>-<target>.tar.gz`, or `.zip` on
Windows) hold the binary, this file, the license and the third-party notices
(and on Linux the udev rule).

| Target | Systems |
|---|---|
| `x86_64-unknown-linux-gnu` | Linux on PCs (glibc 2.28 or newer) |
| `aarch64-unknown-linux-gnu` | 64-bit Raspberry Pi OS and other 64-bit ARM Linux |
| `universal-apple-darwin` | macOS 11 or newer, Apple silicon and Intel |
| `x86_64-pc-windows-msvc` | Windows 10 and 11 |

Put the binary somewhere on your `PATH`.

- **Linux:** install the udev rule so your user can open the bridge
  ([linux.md](linux.md)):

  ```sh
  sudo cp 70-hardwario-tower-bridge.rules /etc/udev/rules.d/
  sudo udevadm control --reload && sudo udevadm trigger
  ```

- **macOS:** the binary is not notarized yet. Clear the quarantine flag once:
  `xattr -d com.apple.quarantine tower-bridge-cli`.

From source (Rust 1.99 or newer; on Linux also `libudev-dev`):

```sh
cargo install --path crates/tower-bridge-cli
```

## Everyday use

```sh
tower-bridge-cli                      # the dashboard (in a terminal)
tower-bridge-cli status               # the bridge, its modules and the latest values
tower-bridge-cli watch                # a line per reading; Ctrl+C stops
tower-bridge-cli --json watch | jq .  # the same as NDJSON
tower-bridge-cli relay                # the Relay Module's state
tower-bridge-cli relay true           # switch it to TRUE (or false)
tower-bridge-cli relay pulse --ms 3000  # TRUE or FALSE for 3 s, then back
tower-bridge-cli gps                  # position and fix
tower-bridge-cli nfc write hardwario.com/tower
tower-bridge-cli identify             # blink the LED on the bridge
tower-bridge-cli export --last 7d -o week.xlsx
tower-bridge-cli export --from 2026-10-01 --to 2026-10-08 > october.csv
tower-bridge-cli alerts               # limits, active alerts, recent alert log
tower-bridge-cli config set mqtt.host broker.local
```

`--help` after any command lists its options. `--simulate` uses a simulated
bridge with every module; its data is kept apart from the real data (in
`simulated/` inside the data folder).

### The dashboard

With the simulated bridge (`tower-bridge-cli --simulate`), 100 by 30 characters:

```text
 TOWER Bridge  ● connected over USB   interval 20 s   via this process
┌ Values ──────────────────────────────┐┌ Temperature · last hour ─────────────────────────────────┐
│I²C 0  CO2 Module, Climate Module R2, ││24.9   │                                           ┌─────┐│
│▸Temperature      24.4 °C  ▁▁▅▄▅▄▄▇▆█ ││       │                                     ⡀⣀⣀⣀⣀⣀│I²C 0││
│ Humidity          41.1 %  ▅▁▆▆▅▆▆█▇▇ ││       │⣀⡀⣀⣀⢀⣀⣀⣀⣀⣀⣀⣀⣀⣀⣀⠤⠤⡠⠤⠤⠤⠤⠤⠤⠤⠒⠒⠒⠒⠒⠚⠒⠉⠉⠉⠉⠉⠈     │I²C 1││
│ Illuminance       738 lx  ▄▅▆▅▂▁█▇▇▇ ││       │ ⠈⠈ ⠁                                      └─────┘│
│ Pressure       999.8 hPa  ▁▃▇▇▂▇█▂▄▇ ││       │                                                  │
│ CO₂                    …             ││       │                                                  │
│ VOC                    …             ││       │                                                  │
│I²C 1  Barometer Tag, Humidity Tag, Lu││22.4 °C│                                                  │
│ Temperature      20.5 °C  ▁▂▃▅▆▆▅▅█▆ ││       │                                                  │
│ Humidity          48.9 %  ▁▂▄▄▅▅▆▅█▅ ││       │                                                  │
│ Illuminance      1338 lx  ▆▄▁▆▇▆▃▇▃█ ││       │                                                  │
│ Pressure       999.9 hPa  ▄▅▇▄█▄▁▇▄▇ ││       │                                                ⢀⢀│
│                                      ││       │⠑⠑⠉⠊⠒⠒⠒⠒⠒⠒⠒⠒⠒⠒⠒⠒⠒⠒⠒⠒⠔⠒⠔⠒⠒⠢⠢⠒⠒⠒⠒⠒⠒⠒⠒⠒⠒⠒⠒⠊⠓⠉⠉⠉⠉⠉⠉⠉⠁⠁│
│                                      ││19.8   │                                                  │
│                                      ││       └──────────────────────────────────────────────────│
│                                      ││12:08:59                    12:38:59                   now│
└──────────────────────────────────────┘└──────────────────────────────────────────────────────────┘
┌ Relay ────────────────────────┐┌ GPS ───────────────────────────┐┌ NFC ──────────────────────────┐
│● TRUE                         ││50.77116, 15.05633              ││No URL                         │
│t TRUE  f FALSE  p pulse       ││±2 m, 9 satellites              ││n write a URL                  │
└───────────────────────────────┘└────────────────────────────────┘└───────────────────────────────┘
┌ Log ─────────────────────────────────────────────────────────────────────────────────────────────┐
│13:08:43  INFO bridge connected over USB                                                          │
│13:08:46  INFO GPS receiver on channel 1 ready                                                    │
│13:08:51  INFO relay switched true                                                                │
│13:08:51  INFO TOWER GPS Module found on I2C channel 1                                            │
└──────────────────────────────────────────────────────────────────────────────────────────────────┘
 ↑↓  value  1 2 3  span  t f p  relay  i  identify  r  rescan  n  NFC  q  quit
```

The keys are listed in the footer:

| Key | Action |
|---|---|
| ↑ ↓ (or k j) | Choose the value whose chart is shown |
| 1 2 3 | Chart of the last hour, 24 hours or 7 days |
| t f | Switch the relay to TRUE or FALSE |
| p | Pulse the relay (the opposite state, for the configured length) |
| i | Blink the LED on the bridge |
| r | Look for modules again |
| n | Write a URL to the NFC Tag (Enter writes, Esc cancels) |
| q, Esc, F10, Ctrl+C | Quit |

The dashboard needs a terminal of at least 60 by 16 characters.

## One data folder, one instance

The app and the CLI keep their settings and history in one folder:

| System | Folder |
|---|---|
| macOS | `~/Library/Application Support/com.hardwario.tower-bridge` |
| Windows | `%LOCALAPPDATA%\com.hardwario.tower-bridge`, or `data` next to the exe |
| Linux | `~/.local/share/com.hardwario.tower-bridge` |

`--data-dir` (or `TOWER_BRIDGE_DATA_DIR`) chooses another folder.
`tower-bridge-cli config path` shows the settings file.

Only one program can open the bridge. One instance per data folder runs the
service and holds the bridge: the desktop app, `tower-bridge-cli run`, or the
dashboard. The other commands work like this:

- **Nothing runs:** they open the bridge themselves for a moment. Nothing is
  recorded or published.
- **An instance runs with its API server on:** they go through its API
  server. Changes such as switching the relay need **Allow changes**.
- **An instance runs without its API server:** they stop with exit code 3 and
  say what holds the bridge. Turn the API server on (in the app's Settings,
  or with `config set api.enabled true` while nothing runs).

`export`, `alerts` and `config get` read the folder directly and work at any
time. `config set` refuses while an instance runs, because that instance
would overwrite the change.

## Running as a service

`tower-bridge-cli run` records the history, checks the alert limits, publishes
to MQTT and serves the API, as the settings say. It logs to stderr. It stops
on Ctrl+C, SIGTERM or SIGHUP: it saves the sensors' state, sends MQTT
`offline` and switches the LED off. Configure it before the first start, for
example:

```sh
tower-bridge-cli config set mqtt.enabled true
tower-bridge-cli config set mqtt.host broker.local
tower-bridge-cli config set api.enabled true
```

### systemd (Linux, Raspberry Pi)

A user service, which runs while you are logged in (or always, after
`loginctl enable-linger $USER`). Put this in
`~/.config/systemd/user/tower-bridge.service`:

```ini
[Unit]
Description=TOWER Bridge
After=network-online.target

[Service]
ExecStart=%h/.local/bin/tower-bridge-cli run
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
```

Then:

```sh
systemctl --user daemon-reload
systemctl --user enable --now tower-bridge
journalctl --user -u tower-bridge -f
```

### launchd (macOS)

Put this in `~/Library/LaunchAgents/com.hardwario.tower-bridge-cli.plist`,
with the path of your binary:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>com.hardwario.tower-bridge-cli</string>
  <key>ProgramArguments</key>
  <array>
    <string>/usr/local/bin/tower-bridge-cli</string>
    <string>run</string>
  </array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>StandardErrorPath</key><string>/tmp/tower-bridge-cli.log</string>
</dict>
</plist>
```

Then `launchctl load ~/Library/LaunchAgents/com.hardwario.tower-bridge-cli.plist`.

### Windows

Start `tower-bridge-cli.exe run` from a Task Scheduler task with the trigger
"At log on".

## MCP for AI assistants

`tower-bridge-cli mcp` is an MCP server over stdin and stdout. It forwards to
the running instance's API server (see [api.md](api.md)):

```json
{ "mcpServers": { "tower-bridge": { "command": "tower-bridge-cli", "args": ["mcp"] } } }
```

## JSON output

With `--json`:

- `status` prints `{"via": ..., "status": {"device", "latest", "relay", "nfc",
  "gps", "active_alerts", "mqtt", "sample_interval_ms"}}`;
- `relay`, `gps` and `nfc` print their objects as in the REST API
  ([api.md](api.md));
- `watch` prints one object per line. Readings have `"type": "reading"`;
  changes of the bridge, the relay, the NFC Tag and alerts have `"type":
  "device"`, `"relay"`, `"nfc"` and `"alert"`, and the GPS position every 5 s
  `"type": "gps"`.

Values are in SI units, as in the REST API: °C, %, lx, Pa, ppm, ppb. Times are
Unix milliseconds.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Done |
| 1 | Error |
| 2 | Usage: an unknown command or option, an invalid value, or no terminal for the dashboard |
| 3 | Busy: another instance or program holds the bridge, or a setting cannot change while the service runs |
| 4 | Not found: no bridge, or not the module the command needs, or no history yet |

Errors go to stderr as `error: ...`, often with a `hint: ...` line that says
the way out.

## Bringing up hardware

`tower-bridge-cli toolbox` holds the low-level tools from the bring-up of new
modules. They open the bridge directly, without the service:

| Tool | Does |
|---|---|
| `scan` | List the bridges the computer sees |
| `probe` | Find the sensors and print a measurement in the format of hardwario/enmon |
| `led on`, `led off` | Switch the bridge LED |
| `i2c-scan` | List the I²C addresses that answer, per channel |
| `i2c-identify` | Name the sensor chips on each channel |
| `i2c-read`, `i2c-write` | Raw transfers, e.g. `i2c-read 0x38 0x00 --len 2` |
| `co2`, `voc`, `gps` | Measure with one module directly |
| `nfc-scan` | Look for an NFC Tag at every address |

`--channel 0|1`, `--backend hid|i2c-dev` and `--trace` (every FT260 report on
stderr) go before the tool, e.g. `tower-bridge-cli toolbox --trace probe`.
