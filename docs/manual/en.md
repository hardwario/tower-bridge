# TOWER Bridge user manual

TOWER Bridge shows the readings of the HARDWARIO TOWER Climate Module and TOWER sensor tags connected through the TOWER Bridge Module, keeps their history and switches the TOWER Relay Module. This manual describes version {version}.

## Introduction {#introduction}

You need:

- **TOWER Bridge Module** connected to the computer over USB.
- **TOWER Climate Module** (revision R1 or R2) for temperature, relative humidity, illuminance and barometric pressure.
- Optionally TOWER sensor tags: **Humidity Tag**, **Temperature Tag**, **Lux Meter Tag** and **Barometer Tag**, alone or next to the Climate Module.
- Optionally a **TOWER Relay Module**, a latching relay you can switch from the app.
- Optionally a **TOWER NFC Tag**, which hands a web address of your choice to a phone that touches it.
- Optionally a **TOWER CO2 Module** (carbon dioxide), a **VOC-LP Tag** (volatile organic compounds) and a **TOWER GPS Module** (position).

The app lives in the menu bar (macOS) or in the system tray (Windows, Linux). It keeps measuring when its window is closed.

## Getting started {#getting-started}

### Install {#install}

- **macOS:** open the DMG file and drag TOWER Bridge to Applications.
- **Windows:** run the EXE file. No installation is needed; the window uses WebView2, which is part of Windows 11.
- **Linux:** make the AppImage executable and run it. Install the udev rule from the project so the app can open the bridge without administrator rights.

### Connect the hardware {#connect}

Plug the Climate Module, and the Relay Module if you have one, into the Bridge Module and connect it over USB. The app finds the modules within a few seconds. After changing modules while the app runs, press **Rescan** on the Dashboard.

![How the hardware connects](diagrams/hardware.svg)

The Relay Module and the tags are optional, as the dashed outlines show.

The Bridge Module has two I²C channels, and sensors can be on both, for example the Climate Module indoors on channel 0 and tags outdoors on channel 1. The app looks on both channels and finds a tag plugged in while it runs within a minute.

### Menu bar and tray {#tray}

The icon shows the current value of the measurement chosen in the settings. Its menu opens the dashboard, lists all readings and the connection status, switches **Open at Login** and MQTT on or off, and quits the app. Closing the dashboard window keeps the app running. When a quantity is measured on both channels, the menu shows both values, channel 0 first.

## Dashboard {#dashboard}

The Dashboard shows the Relay Module, the latest value of each measurement with its chart, and the TOWER Bridge Module. **Last update** at the top tells when the latest measurement was taken. Next to it, **Interval** sets how often the sensors are measured, from 5 seconds to 3 minutes (20 seconds by default).

![The Dashboard](images/en/dashboard.webp)

### Charts {#charts}

![Parts of a chart panel](diagrams/chart-card.svg)

1. **Colour:** picks the line colour.
2. **Data source:** point at the ⓘ icon to see the module, the I²C channel, the address and the sensor behind the values.
3. **Reset:** returns to the selected time span.
4. **Maximize:** enlarges the chart; press it again, or <kbd>Esc</kbd>, to restore it.
5. **Move:** drag the panel to a new place; the order is remembered.
6. **Chart:** point at it to see the exact value and time.
7. **Min, avg, max:** for the visible range. Click one to show or hide its line in the chart.

Choose the time span above the charts, from 1 hour to 365 days. All charts on a page move together.

| To | Do this |
|---|---|
| Pan | Drag the chart left or right |
| Zoom | <kbd>Ctrl</kbd> + scroll (<kbd>⌘</kbd> + scroll on macOS), or pinch on a touchpad |
| Zoom into a range | <kbd>Shift</kbd> + drag |
| Return to the time span | Reset icon (4) |

A red edge at the top of a panel means the value is outside its alert limits; the dashed red line in the chart shows the limit.

When the same quantity is measured on both channels, for example temperature by the Climate Module and by the Humidity Tag, its panel shows both values, a line for each channel in its own colour, and a row with min, avg and max for each below the chart. The coloured dot at the start of a row picks that line's colour, and the column heads switch the min, avg and max lines on or off. Click a row's name, for example *Humidity Tag*, to hide its line from the chart; the row turns grey, and another click brings the line back. One line always stays.

![A panel with two sources](images/en/chart.webp)

### Relay Module {#relay}

![The Relay Module panel](images/en/relay.webp)

The relay is latching, so it keeps its position without power. Its state is **TRUE** (green LED on the module) or **FALSE** (red LED). After a power cut the app cannot know the position; the state is **Unknown** until you switch the relay once.

![Relay states](diagrams/relay-states.svg)

- **Activate TRUE** and **Activate FALSE** switch the relay.
- **Pulse TRUE** and **Pulse FALSE** switch to that state for the **Pulse duration** (0.5 s to 1 hour, 2 s by default) and then back. The button fills while the pulse runs. Click it again to end the pulse early.

![A pulse to TRUE](diagrams/relay-pulse.svg)

### CO₂ and VOC {#air}

CO₂ and VOC panels appear when a TOWER CO2 Module or a VOC-LP Tag is connected.

- **CO₂** in ppm. The first value comes about a minute after the start, while the module charges its capacitor; then it is measured with every measurement, at most every 16 seconds. The value is noise filtered and corrected for air pressure with the barometer, if there is one. The sensor corrects its own baseline every 8 days, so let fresh air into the room now and then; after a new start the values settle within a few days.
- **VOC** (total volatile organic compounds) in ppb, measured every 30 seconds. The first values come after about five minutes of warm-up. With a humidity sensor on the same channel, the value is corrected for humidity.

Both sensors keep what they learned when the app restarts.

### GPS {#gps}

![The GPS card](images/en/gps.webp)

The GPS card appears when a TOWER GPS Module is connected. It shows whether the receiver has a position fix and how many satellites it uses; with a fix also the position, its accuracy, the altitude above sea level and the time from the satellites. The map beside it works without the internet: countries, rivers, lakes and towns, with the position as a red dot. Drag it to look around, zoom with the + and − buttons, Ctrl/⌘ and the scroll wheel, or a pinch; once the map has moved, a third button brings the position back to the middle. For streets, **Show on map** opens the position on OpenStreetMap. The first fix can take several minutes and needs a view of the sky; the green LED on the module lights up with a fix.

### TOWER NFC Tag {#nfc}

The TOWER NFC Tag holds a web address. A phone that touches it opens the address, for example a page about the room or the device. The card appears when the tag is connected and shows the address on the tag. Type a new one and press **Write**; without `https://` it is added for you. Addresses up to 240 characters fit.

### TOWER Bridge Module {#bridge}

Shows the modules found on each I²C channel (the Relay Module included). **Identify** blinks the LED on the Bridge Module. **Rescan** looks for modules again. With more than one Bridge Module plugged in, the card warns you: the app reads only one of them, the one whose LED blinks on **Identify**. Unplug the others, or readings from different setups can end up mixed. The card is always the last one on the Dashboard.

## Data {#data}

Shows the stored readings for the same time spans as the Dashboard, 1 hour up to 365 days, or for a custom range, as a table with the newest first. Each row averages a stretch of time that grows with the range; the line under the table says how long. **Export CSV** and **Export XLSX** save every individual reading of the selected range; XLSX files use your date and time format. Exports have one row per measurement and I²C channel. The charts are on the Dashboard.

![Data](images/en/data.webp)

## Alerts {#alerts}

Set a lower limit (**Below**) and an upper limit (**Above**) for each measurement. When a value leaves its range, you get a notification. Limits apply on both channels; when a quantity is measured on both, the notification names the module.

![Alert limits](images/en/alerts.webp)

- **Hysteresis:** how far the value must return inside the limit before the alert ends, so that a value close to the limit does not raise alerts again and again.
- **Repeat after:** the shortest time between two notifications for the same limit.
- **Notify when back to normal:** also tells you when the value is in range again.
- **Bridge disconnected:** notifies you when no measurement arrives for a minute.

![How hysteresis works](diagrams/hysteresis.svg)

In the diagram the value rises above the limit: the alert starts and you get a notification (red bell). A dip into the hysteresis band does not end it. Only when the value falls below the band does the alert end, with a notification if **Notify when back to normal** is on (green bell).

**Send test** under Test notification checks that notifications reach you. Recent alerts are listed at the bottom.

## Settings {#settings}

- **General:** appearance (system, light, dark), language, date format, time format, decimal separator (a point or a comma; Automatic follows the language), temperature unit, the value shown next to the menu bar or tray icon, and open at login.
- **Measurement:** the sea-level pressure for the altitude estimate, and the I²C channels to look for sensors on (Automatic uses both).
- **Data:** how long readings are kept and where the data folder is.

### MQTT {#mqtt}

![How other software connects](diagrams/integrations.svg)

MQTT is off by default. Switched on, the app publishes every reading to your own MQTT broker, for example Mosquitto or the one in Home Assistant; it does not run a broker itself. Enter the **Broker address**, and the port, TLS and login if the broker needs them. Changes apply at once, and the app reconnects by itself when the broker goes away. The status under the switch says whether it is connected.

Every reading is published as a retained message under the topic prefix (`tower` by default), with the I²C channel at the end: `tower/temperature/0`, `tower/humidity/0`, `tower/illuminance/0`, `tower/pressure/0` and `tower/altitude/0`. Sensors on channel 1 publish on `tower/temperature/1` and so on. A TOWER CO2 Module publishes on `tower/co2/0` (ppm), a VOC-LP Tag on `tower/voc/0` (ppb), and a TOWER GPS Module on `tower/gps/0/latitude`, `/longitude`, `/altitude` and `/satellites`. The relay state is published on `tower/relay/0` (the channel of the Relay Module) as `true`, `false` or `null` while it is unknown. `tower/status` says `online` while the app publishes and `offline` when it stops.

With **Allow relay control** switched on, the relay accepts `true` or `false` on `tower/relay/0/set` and a pulse on `tower/relay/0/pulse`.

### API and MCP {#api}

The optional local server offers a REST API, Prometheus metrics and an MCP server for AI assistants; it is off by default. Changes such as alert limits or switching the relay need **Allow changes**. Other computers must send the access token shown in the settings.

## Troubleshooting {#troubleshooting}

- **Bridge not connected:** check the USB cable. Only one program can use the bridge at a time, so quit other tools that may hold it.
- **No permission (Linux):** install the [udev rule](#install) and reconnect the bridge.
- **No sensors found:** check that the modules and tags are seated properly and press **Rescan**. Leave the I²C channels on Automatic, or set the channel they are connected to.
- **Relay state unknown:** switch the relay once with Activate TRUE or Activate FALSE.
- **No notifications:** allow notifications for TOWER Bridge in the system settings and use **Send test**.

## About {#about}

TOWER Bridge is open source under the MIT license, © HARDWARIO a.s. Documentation of the TOWER platform is at [docs.hardwario.com/tower](https://docs.hardwario.com/tower/); the hardware is sold in the [HARDWARIO online store](https://www.hardwario.store/).
