# Built-in MQTT broker

TOWER Bridge can run an MQTT 3.1.1 broker (rumqttd) inside the app. It is off
by default; enable it in **Settings → MQTT broker** or from the tray menu.

- Listens on `127.0.0.1:1883` by default. *Allow network access* listens on all
  interfaces; set a username and password in that case.
- Every reading is published as a **retained** QoS 0 message, so a new
  subscriber immediately gets the latest values.
- Changing the port, network access or credentials, or turning the broker off,
  takes effect after an app restart (the app offers to restart itself).

## Topics

Every reading is published under the *Topic prefix* setting (default `tower`):

| Topic | Payload | Unit |
|---|---|---|
| `tower/temperature` | `23.46` | °C |
| `tower/humidity` | `45.2` | % |
| `tower/illuminance` | `320.5` | lx |
| `tower/pressure` | `98765.25` | Pa |
| `tower/altitude` | `312.4` | m |

Payloads are plain numbers. Temperature is always published in °C regardless of
the display unit. The prefix may have several levels, e.g. `home/office/tower`.

With a TOWER Relay Module connected, its state is published too:

| Topic | Payload |
|---|---|
| `tower/relay` | `true`, `false` or `null` (unknown) |

The relay is latching and cannot be read back, so the state is `null` until
it has been switched. The topic is cleared when the module disappears.

## Relay control

Turn on *Allow relay control* in the MQTT settings (off by default). Then:

| Topic | Payload | Effect |
|---|---|---|
| `tower/relay/set` | `true` or `false` | Switch the relay (ends a running pulse) |
| `tower/relay/pulse` | empty | Switch to the other state for the configured time (default 2 s), then back |
| `tower/relay/pulse` | `1500` | The same for 1500 ms |
| `tower/relay/pulse` | `true` or `false` | Pulse to that state (back to the other one afterwards) |
| `tower/relay/pulse` | `{"state": false, "duration_ms": 3000}` | Both fields are optional |

Pulses last 0.5 s to 1 h. Do not publish commands as retained messages; the
broker never replays them to the app anyway. Invalid commands are ignored and
logged.

## Examples

```sh
mosquitto_sub -h 127.0.0.1 -t 'tower/#' -v
mosquitto_pub -h 127.0.0.1 -t tower/relay/set -m false
mosquitto_pub -h 127.0.0.1 -t tower/relay/pulse -m 2000
```

Node-RED: add an *mqtt in* node subscribed to `tower/#`.
