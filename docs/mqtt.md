# MQTT

TOWER Bridge publishes every reading to your MQTT broker, for example
Mosquitto or the one in Home Assistant. The app is a client: it connects to the
broker and publishes there; it does not run a broker itself. MQTT is off by
default; turn it on in **Settings → MQTT** or from the tray menu.

Every reading is published as a **retained** QoS 0 message, so a new
subscriber immediately gets the latest values, and relay commands are accepted
on the same topics (see below).

## Connection

| Setting | Meaning |
|---|---|
| Broker address | Host name or IP address, e.g. `homeassistant.local` |
| Port | 1883, or usually 8883 with TLS |
| TLS | Encrypted connection, checked against the system's root certificates |
| Username, Password | If the broker asks for a login |
| Client ID | How the broker tells the app apart; chosen once (`tower-bridge-` and 8 hex digits) |

- Settings take effect at once, without a restart.
- The app reconnects by itself when the broker goes away (after 1 s, then
  waiting longer up to 30 s) and publishes all current values again on every
  connect, since the broker may have lost them.
- `tower/status` says `online` while the app publishes and `offline` when it
  stops. The broker also sets `offline` (the app's last will) when the app
  vanishes without saying goodbye. Home Assistant can use it as the
  availability topic.
- The status under *Enable MQTT* shows whether the app is connected, and why
  not.

No broker yet? Mosquitto is a small install (`brew install mosquitto`,
`apt install mosquitto`, or the Windows installer from mosquitto.org); Home
Assistant has one as the *Mosquitto broker* add-on.

## Topics

Every reading is published under the *Topic prefix* setting (default `tower`).
The last level is the I²C channel of the Bridge Module the sensor is
connected to, `0` or `1`:

| Topic | Payload | Unit |
|---|---|---|
| `tower/temperature/0` | `23.46` | °C |
| `tower/humidity/0` | `45.2` | % |
| `tower/illuminance/0` | `320.5` | lx |
| `tower/pressure/0` | `98765.25` | Pa |
| `tower/altitude/0` | `312.4` | m |

With sensors on both channels, for example a Climate Module on channel 0 and
the Humidity, Lux Meter and Barometer Tags on channel 1, the tags' values come
on `tower/temperature/1`, `tower/humidity/1` and so on. The channel is always
part of the topic, so topics do not change when a second sensor is plugged in.
Topics name what is measured, not the hardware: swapping a tag for a Climate
Module on the same channel keeps them.

Useful subscriptions: `tower/temperature/+` gets every temperature,
`tower/+/1` everything on channel 1.

Payloads are plain numbers. Temperature is always published in °C regardless of
the display unit. The prefix may have several levels, e.g. `home/office/tower`.

With a TOWER Relay Module connected, its state is published on the topic of
its channel:

| Topic | Payload |
|---|---|
| `tower/relay/0` | `true`, `false` or `null` (unknown) |
| `tower/status` | `online` or `offline` (the app is publishing or not) |

The relay is latching and cannot be read back, so the state is `null` until
it has been switched. The topic is cleared when the module disappears.

## Relay control

Turn on *Allow relay control* in the MQTT settings (off by default). Then:

| Topic | Payload | Effect |
|---|---|---|
| `tower/relay/0/set` | `true` or `false` | Switch the relay (ends a running pulse) |
| `tower/relay/0/pulse` | empty | Switch to the other state for the configured time (default 2 s), then back; while a pulse runs, extend it |
| `tower/relay/0/pulse` | `1500` | The same for 1500 ms |
| `tower/relay/0/pulse` | `true` or `false` | Pulse to that state (back to the other one afterwards) |
| `tower/relay/0/pulse` | `{"state": false, "duration_ms": 3000}` | Both fields are optional |

Use the channel the Relay Module is on; commands for a channel without a relay
are ignored. Pulses last 0.5 s to 1 h. Commands published with the retain flag
are ignored, so a stored command never fires later (for example after a prefix
change). Invalid commands, including JSON with unknown fields, are ignored and
logged.

## Examples

With the broker on `homeassistant.local`:

```sh
mosquitto_sub -h homeassistant.local -t 'tower/#' -v
mosquitto_sub -h homeassistant.local -t 'tower/temperature/+' -v
mosquitto_pub -h homeassistant.local -t tower/relay/0/set -m false
mosquitto_pub -h homeassistant.local -t tower/relay/0/pulse -m 2000
```

Node-RED: add an *mqtt in* node subscribed to `tower/#`.
