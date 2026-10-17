# Hardware reference

TOWER Bridge talks to the HARDWARIO **TOWER Bridge Module** over USB HID and
reads the sensors of the **TOWER Climate Module** and the TOWER sensor tags
(Humidity, Temperature, Lux Meter and Barometer Tag) over I2C, on both channels
of the bridge's I2C switch. It also switches the **TOWER Relay Module**. The protocol was
ported from [hardwario/enmon](https://github.com/hardwario/enmon) (MIT) and the
Climate Module layout from
[hardwario/twr-sdk](https://github.com/hardwario/twr-sdk)
(`twr_module_climate.c`).

## Bridge Module (FTDI FT260)

| Item | Value |
|---|---|
| USB VID:PID | `0403:6030` |
| Interface | 0 (I2C); on Windows the path contains `&mi_00` |
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

## Sensor detection

Both switch channels are probed part by part (*Settings → I²C channels* can
limit this to one channel):

| Part | Address | Probe |
|---|---|---|
| OPT3001 | `0x44`, then `0x45` (Lux Meter Tag) | device ID register `0x7F` reads `0x3001` |
| SHT30 | `0x45` | soft reset `30 A2` acknowledged |
| SHT20 | `0x40` | soft reset `FE` acknowledged |
| TMP112 | `0x48`, then `0x49` (Temperature Tag) | configuration register `0x01` readable |
| MPL3115A2 | `0x60` | WHO_AM_I register `0x0C` reads `0xC4` |

The OPT3001 is probed first and by its ID: an OPT3001 at `0x45` would also
acknowledge the SHT30 soft reset. One part of each kind is used per channel,
the one at the default address when both answer.

The SHT humidity sensors are probed with command writes: a read probe would not
work, since the SHT3x NACKs reads when no measurement is pending.

The parts on a channel are named after the module that carries them:

| Parts on the channel | Module |
|---|---|
| SHT30 with OPT3001 (at `0x44`) or MPL3115A2 | Climate Module R2 |
| SHT20 with TMP112 (at `0x48`) | Climate Module R1 |
| otherwise SHT20 or SHT30 | Humidity Tag |
| otherwise TMP112, and a TMP112 at `0x49` | Temperature Tag |
| otherwise OPT3001, and an OPT3001 at `0x45` | Lux Meter Tag |
| otherwise MPL3115A2 | Barometer Tag |

Temperature comes from the SHT30, else the TMP112, else the SHT20 (the Humidity
Tag measures both); humidity from the SHT30, else the SHT20. Checked on real
hardware: the Humidity Tag (SHT20), Lux Meter Tag (OPT3001) and Barometer Tag
(MPL3115A2) on channel 1 next to a Climate Module R1 on channel 0.

A part that fails three measurements in a row is taken as unplugged. Channels
are probed again every minute, so a tag plugged in later is found, and
**Rescan** probes right away.

### Measurement cycle

All conversions on a channel are started together and collected as they
finish, so a channel takes about one second (enmon: about 6.5 s). The channels
are measured one after the other, because the same addresses are used on both;
their readings share the cycle's timestamp.

| Time | Action |
|---|---|
| t0 | SHT30 `24 00` (R2) or TMP112 `01 E1 A0` + SHT20 `F5` (R1), or SHT20 `F3` alone (Humidity Tag); OPT3001 config `CA 10`; MPL3115A2 CTRL_REG1 `3A` |
| +100 ms | SHT30: read 6 bytes, check CRC-8 (poly `0x31`, init `0xFF`). TMP112: wait for OS bit, read reg 0. SHT20: read 3 bytes, check CRC (init `0x00`); after a temperature, trigger `F5` and read humidity about 30 ms later |
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

## TOWER NFC Tag

An NXP NT3H2111 (NTAG I²C plus, 1k) with its antenna (schematic rev 1.3). The
field detect output reaches the tag connector's INT pin through a transistor;
the app does not use it.

| Address | Where it comes from |
|---|---|
| `0x08` | HARDWARIO tags (as twr-sdk expects); verified on a real tag |
| `0x55` | the chip's factory default |

The address is stored in the chip (byte 0 of block 0), so it survives power
cycles. The app looks at both, on both switch channels, on connect, on
*Rescan* and every minute. A bare one-byte read does not find the chip, so
`i2c-scan` misses it; `nfc --scan` sets the memory pointer first and tries
every address.

Over I²C the memory is read and written in 16-byte blocks: write the block
number, STOP, then read 16 bytes (no repeated start). Writing a block takes
about 5 ms of EEPROM programming, during which the chip does not acknowledge.

| Block | Content |
|---|---|
| `0x00` | I²C address (reads back as the NXP ID `0x04`), serial number, static locks, capability container `E1 10 6D 00` |
| `0x01` to `0x37` | user memory: the NDEF message as a TLV, `03 <length> <message> FE` |
| `0x38` and up | dynamic locks and configuration |

The app never writes block 0 (a wrong byte there moves the chip to another
address or locks it for good) and nothing past the user memory. A URL is
written as one URI record (`D1 01 <length> 55 <prefix code> <rest>`, prefix
codes as in the NFC Forum URI record type, at most 240 bytes) and read back to
verify. A tag reads, for example: block 1 `03 12 D1 01 0E 55 02 62 69 67 63 6C
6F 77 6E 2E`, block 2 `63 6F 6D 2F FE`, which is `https://www.bigclown.com/`.

## TOWER CO2 Module

A Senseair LP8 (schematic rev 1.4) with its own supply, a storage capacitor
for the lamp, and a UART. The bridge reaches it through an NXP SC16IS740 I²C
UART at `0x4D` (13.56 MHz crystal, divisor 88: 9630 baud, close enough to the
LP8's 9600 8N2) and a TCA9534A at `0x38`. The app finds the module by the
expander's configuration register and the UART's scratchpad register, on both
channels, on connect, on *Rescan* and every minute.

The expander's output latch stays 0, so a pin switched to output pulls its
line low (on) and an input lets the pull-up switch it off:

| Pin | Function |
|---|---|
| P1 | capacitor charging |
| P2 | sensor supply VDD2 |
| P3 | sensor enable |
| P6 | SC16IS740 reset, active low |
| P7 | RDY from the LP8 (input) |

Configuration register values: `0xEE` idle, `0xE8` charging with VDD2 on,
`0xE0` the sensor enabled as well.

One measurement is one power cycle of the sensor:

1. Enable (`0xE0`), wait 150 ms, wait for RDY low (within 500 ms of power on).
2. Write the calculation control, the 23-byte sensor state from the previous
   measurement and the ambient pressure (0.1 hPa) to RAM `0x80`:
   `FE 41 00 80 1A <control> <state> <pressure> <crc>`. The LP8 answers
   `FE 41 81 E0`.
3. Wait for RDY high (within 1 s of power on), then read 44 bytes from `0x80`:
   `FE 44 00 80 2C 79 39`, 49 bytes back.
4. Back to `0xE8`: the sensor is off, the capacitor keeps charging.

The CRC is Modbus CRC-16, low byte first. Calculation control `0x10` starts
over (no state yet), `0x20` is the normal sequential measurement, `0x70` adds
an automatic baseline correction (ABC), which the host asks for every 8 days.
The reported value is the filtered, pressure compensated concentration
(`0xAA`), the one the datasheet states the accuracy for; the pressure comes
from a barometer on the same bridge, or the LP8's default 1012.4 hPa. A
measurement takes about 0.7 s, and measurements are at least 16 s apart, since
the residual heat of the lamp spoils the next one. Error bits in status 0
(`0xA7`) and status 1 (`0xA6`) that call for it make the app start over with
control `0x10`; a capacitor below its limit postpones the next measurement.

twr-sdk charges the capacitor for 5 s before every measurement and switches
everything off afterwards, as a battery device must. The bridge runs on USB,
so the app charges for a minute after start and then keeps charging, and a
measurement can begin at any time. The sensor state is stored in
`lp8-channel<N>.txt` in the data folder after every measurement and reused
after a restart if it is less than a week old, so the filters and the ABC
baseline carry on. The datasheet is "LP8 sensor module specifications and
integration guideline" v1.02.

## TOWER VOC-LP Tag

A Sensirion SGPC3 at `0x58` (schematic rev 1.0). The app reads its feature set
(`20 2F`, the tag answers `10 06`: product SGPC3, feature set 6), switches it
to ultra-low power (`20 9F 00 00`), starts the algorithm with
`init_preheat` (`20 AE`), and measures TVOC (`20 08`) every 30 s on a fixed
grid, as the algorithm expects in that mode. Every word carries a CRC-8
(polynomial 0x31, init 0xFF).

Before each measurement it sets the absolute humidity (`20 61`, g/m³ in 8.8
fixed point) from a thermometer and hygrometer on the same channel, or 0 to
switch the compensation off. The sensor reads 0 ppb while it preheats; the app
ignores zeros during the first five minutes. The baseline the sensor learns is
read every hour (`20 15`), stored in `sgpc3-channel<N>.txt`, and written back
(`20 1E`) after a restart if it is less than a week old.

twr-sdk measured every 30 s without leaving the default low-power mode, whose
algorithm expects a measurement every 2 s.

## TOWER GPS Module

A u-blox SAM-M8Q at `0x42` (schematic rev 1.2) and a TCA9534 at `0x21`:

| Pin | Function |
|---|---|
| P0 | receiver supply |
| P3 | red LED |
| P5 | green LED, lit while there is a fix |
| P7 | backup supply, kept on so the receiver finds a fix faster after a restart |

twr-sdk reads the NMEA stream the receiver sends on I²C by default. Over the
FT260 every transfer costs about 25 ms, so the app sends CFG-PRT to make the
receiver output only UBX on I²C, and polls one UBX-NAV-PVT every 5 s: fix,
position, altitude above sea level, horizontal accuracy, satellites and UTC
time in one 100-byte message. Register `0xFD` and `0xFE` hold the number of
bytes waiting, `0xFF` the stream, read in 60-byte chunks.

The address `0x21` is not unique to this module, so the app takes the
expander for a GPS Module only once the receiver answers at `0x42`; otherwise
it puts the expander back to all inputs.

## Diagnostics CLI

`tower-bridge-cli` validates hardware support without the desktop app:

```sh
cargo run -p tower-bridge-cli -- scan            # list bridges and interfaces
cargo run -p tower-bridge-cli -- probe --trace   # find the sensors, one cycle per channel, hex dump of every report
cargo run -p tower-bridge-cli -- watch           # continuous, with reconnect
cargo run -p tower-bridge-cli -- i2c-scan        # addresses per channel and in front of the switch
cargo run -p tower-bridge-cli -- i2c-identify    # name the sensor chips on each channel
cargo run -p tower-bridge-cli -- led on
cargo run -p tower-bridge-cli -- relay [true|false]  # find the Relay Module, show or switch it
cargo run -p tower-bridge-cli -- nfc [URL]       # find the TOWER NFC Tag, show its URL or write a new one
cargo run -p tower-bridge-cli -- nfc --scan      # look for a TOWER NFC Tag at every address
cargo run -p tower-bridge-cli -- co2             # find the TOWER CO2 Module, charge for a minute, measure 3 times
cargo run -p tower-bridge-cli -- co2 --charge 5 --pressure 990 --state lp8.txt  # shorter charge, a pressure, a kept state
cargo run -p tower-bridge-cli -- voc             # find the VOC-LP Tag and measure every 30 s
cargo run -p tower-bridge-cli -- gps             # find the TOWER GPS Module and show the position every 5 s
cargo run -p tower-bridge-cli -- i2c-read 0x38 0x03 --len 1   # write a register, STOP, read
cargo run -p tower-bridge-cli -- i2c-write 0x38 0x03 0xff     # write bytes on --channel (default 0)
```

`probe` prints the same lines as enmon (`@SENSOR: "Temperature",27.20`).
Add `--simulate` to any command to use the simulated bridge.
