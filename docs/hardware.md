# Hardware reference

TOWER Bridge talks to the HARDWARIO **TOWER Bridge Module** over USB HID and
reads the sensors of the **TOWER Climate Module** over I2C. It also switches
the **TOWER Relay Module**. The protocol was
ported from [hardwario/enmon](https://github.com/hardwario/enmon) (MIT) and the
Climate Module layout from
[hardwario/twr-sdk](https://github.com/hardwario/twr-sdk)
(`twr_module_climate.c`).

## Bridge Module (FTDI FT260)

| Item | Value |
|---|---|
| USB VID:PID | `0403:6030` |
| Interface | 0 (I2C) — Windows path contains `&mi_00` |
| I2C clock | 100 kHz |
| LED | GPIOH, active high |
| I2C switch | address `0x70`, write `0x01` = channel 0, `0x02` = channel 1 |

All outgoing reports are 64 bytes, zero padded, report ID in byte 0 (Windows
requires the full length; macOS and Linux accept it).

### Initialisation

| Step | Report |
|---|---|
| Chip version | feature get `A0` |
| System status | feature get `A1` |
| 48 MHz clock | feature set `A1 01 02` |
| Enable I2C | feature set `A1 02 01` |
| UART off | feature set `A1 03 00` |
| I2C reset | feature set `A1 20` |
| 100 kHz | feature set `A1 22 64 00` |
| LED | feature set `B0 00 00 80 80` (on) / `B0 00 00 00 80` (off) |

### Transfers

- **Write** (output report): `[D0 + (len-1)/4, addr7, flag, len, data…]`,
  at most 60 data bytes. Flag `0x06` = START+STOP, `0x02` = START only (a
  repeated-start read follows).
- **Read request** (output report): `[C2, addr7, flag, len_lo, len_hi]`, flag
  `0x06` = START+STOP, `0x07` = repeated START+STOP. The data arrives as an input
  report `[D0 + (len-1)/4, len, data…]`.
- **Status** (feature get `C0`, byte 1):

  | Bit | Meaning |
  |---|---|
  | `0x01` | controller busy |
  | `0x02` | error |
  | `0x04` | address NACK |
  | `0x08` | data NACK |
  | `0x10` | arbitration lost |
  | `0x20` | controller idle |
  | `0x40` | bus busy |

  Poll every 10 ms until not busy. After a write without STOP the bus must stay
  busy; otherwise wait until the bus is free and the controller idle. Unlike
  enmon, the poll times out after 200 ms and resets the controller.

## Climate Module

| Revision | Temperature | Humidity | Light | Pressure |
|---|---|---|---|---|
| R1 | TMP112 `0x48` | SHT20 `0x40` | OPT3001 `0x44` | MPL3115A2 `0x60` |
| R2 | SHT30 `0x45` | SHT30 `0x45` | OPT3001 `0x44` | MPL3115A2 `0x60` |

The revision is detected with a command write (SHT30 soft reset `30 A2` at
`0x45` → R2, otherwise SHT20 soft reset `FE` at `0x40` → R1). A read probe would
not work: the SHT3x NACKs reads when no measurement is pending.

### Measurement cycle

All conversions are started together and collected as they finish, so a cycle
takes about one second (enmon: about 6.5 s).

| Time | Action |
|---|---|
| t0 | SHT30 `24 00` (R2) or TMP112 `01 E1 A0` + SHT20 `F5` (R1); OPT3001 config `CA 10`; MPL3115A2 CTRL_REG1 `3A` |
| +100 ms | SHT30: read 6 bytes, check CRC-8 (poly `0x31`, init `0xFF`). TMP112: wait for OS bit, read reg 0. SHT20: read 3 bytes, check CRC (init `0x00`) |
| +550 ms | MPL3115A2: wait for STATUS bit `0x04`, read 5 bytes from `0x01` |
| +880 ms | OPT3001: wait until mode = shutdown and conversion-ready, read reg 0 |

### Conversions

| Sensor | Formula |
|---|---|
| SHT30 | T = −45 + 175·raw/65535 °C; RH = 100·raw/65535 % |
| SHT20 | raw &= ~3; T = −46.85 + 175.72·raw/65536; RH = −6 + 125·raw/65536 |
| TMP112 | (i16 >> 4) · 0.0625 °C |
| OPT3001 | 0.01 · 2^(r >> 12) · (r & 0xFFF) lx |
| MPL3115A2 | (be24 >> 4) / 4 Pa |

Altitude is derived from pressure in software:
`h = 44330.77 · (1 − (p / p0)^0.190263)` with a configurable sea-level pressure
`p0` (default 1013.25 hPa).

## Relay Module

A latching relay (IM41, contacts 5 A / 12 V DC) switched by an H-bridge on a
TCA9534A I/O expander (schematic rev 1.5). Some modules carry a TCA9534
instead: same registers, base address `0x20` instead of `0x38`. The app looks
on both switch channels at `0x3B`, `0x3F`, `0x23` and `0x27`, on connect, on
*Rescan*, and every minute while no relay has been found.

| Pin | Net | Function |
|---|---|---|
| P0 | LED_1 | green LED, state `true` |
| P1 | LED_2 | red LED, state `false` |
| P4 | RELAY_F_H | high side, F direction (P-channel, active low) |
| P5 | RELAY_T_L | low side, T direction (N-channel, active high) |
| P6 | RELAY_T_H | high side, T direction (P-channel, active low) |
| P7 | RELAY_F_L | low side, F direction (N-channel, active high) |

The relay state is `true` for the T direction (as twr-sdk `true`) and `false`
for F, as verified on the module.

| Address | Strapping |
|---|---|
| `0x3B` (`0x23` on TCA9534) | default: A2 low (R6 0 Ω), A1 and A0 high |
| `0x3F` (`0x27` on TCA9534) | R3 fitted, R6 removed (A2 high) |

Registers: `0x01` output, `0x03` configuration (0 = output). Switching:

1. Output `0x50` plus the LED bit (all transistors off), configuration `0x00`.
   Repeated before every switch in case the module lost power.
2. Output `0x31` (`true`) or `0xC2` (`false`): coil pulse, LED updated.
3. After 20 ms, output `0x51` or `0x52` again (idle, LED kept).

Pulses are at least 300 ms apart so that C1 (220 µF, charged through 330 Ω)
can recharge. The contact position cannot be read back. The LED bits in the
output register tell the state after an app restart, as long as the module
stayed powered. After power-up the register reads `0xFF`, so the state shows as
unknown (`null`) until the relay is switched.

A timed pulse (dashboard *Pulse* button, REST, MCP, MQTT) moves the relay to
one state and back after 0.5 s to 1 h (default 2 s). The acquisition thread
times it and does not start a measurement cycle that would delay the return.
If the bridge is unplugged during a pulse, the relay stays switched over.

## Diagnostics CLI

`tower-bridge-cli` validates hardware support without the desktop app:

```sh
cargo run -p tower-bridge-cli -- scan            # list bridges and interfaces
cargo run -p tower-bridge-cli -- probe --trace   # one cycle, hex dump of every report
cargo run -p tower-bridge-cli -- watch           # continuous, with reconnect
cargo run -p tower-bridge-cli -- i2c-scan        # addresses per channel and in front of the switch
cargo run -p tower-bridge-cli -- led on
cargo run -p tower-bridge-cli -- relay [true|false]  # find the Relay Module, show or switch it
```

`probe` prints the same lines as enmon (`@SENSOR: "Temperature",27.20`).
Add `--simulate` to any command to use the simulated bridge.
