// Development preview: runs the dashboard in a plain browser with a mocked
// backend (no Tauri, no hardware). Open http://localhost:1420/preview.html
// Query parameters: ?tab=data|alerts|settings|manual&state=ready|not_found|no_module|permission&action=identify|rescan
//                   &theme=system|light|dark&lang=system|en|cs|sk|de|pl&date=auto|iso|dmy|dmy_slash|mdy&time=auto|h24|h12&decimal=auto|point|comma
//                   &tags=0 (only the Climate Module on channel 0, no sensor tags on channel 1)
//                   &mqtt=off|connecting|no_host (the MQTT connection; connected by default)
//                   &bridges=2 (more than one TOWER Bridge Module plugged in)
//                   &nfc=url|empty|unformatted (an NFC Tag on channel 0)
//                   &co2=1 (a CO2 Module and a VOC-LP Tag on channel 0)  &gps=fix|nofix (a GPS Module)

import { mockIPC } from "@tauri-apps/api/mocks";
import { emit } from "@tauri-apps/api/event";
import type { LanguagePref } from "./lib/i18n/index.svelte";
import type { ThemePref } from "./lib/theme.svelte";
import { METRICS } from "./lib/types";
import type {
  DateFormat,
  DecimalSeparator,
  DeviceState,
  GpsInfo,
  History,
  NfcInfo,
  Reading,
  RelayInfo,
  SensorSource,
  Series,
  Settings,
  Snapshot,
  TimeFormat,
} from "./lib/types";

const params = new URLSearchParams(location.search);
const mqttParam = params.get("mqtt");
const DAY = 86_400_000;
/** A Climate Module on channel 0, and the Humidity, Lux Meter and Barometer Tags on channel 1. */
const CHANNELS = params.get("tags") === "0" ? [0] : [0, 1];
/** ?co2=1: a CO2 Module and a VOC-LP Tag on channel 0. */
const AIR = params.get("co2") === "1";

type Mocked = Record<"temperature" | "humidity" | "illuminance" | "pressure", number> & {
  co2: number | null;
  voc: number | null;
};

function values(ts: number, channel = 0): Mocked {
  if (channel === 1) {
    // The tags are somewhere a bit cooler, damper and brighter.
    const v = values(ts - 1_800_000);
    return {
      temperature: v.temperature - 3.5,
      humidity: Math.min(100, v.humidity + 9),
      illuminance: v.illuminance * 1.8,
      pressure: v.pressure + 12,
      co2: null,
      voc: null,
    };
  }
  const wall = ts / 1000;
  const day = (((wall + 7200) % 86400) / 86400) * Math.PI * 2;
  const drift = Math.sin(wall / 1800);
  const weather = Math.sin(wall / (86400 * 2.3)) + 0.5 * Math.cos(wall / (86400 * 0.9));
  const n = () => Math.random() - 0.5;
  return {
    temperature: 22.3 + 2.2 * Math.sin(day - 2) + 0.35 * drift + 0.6 * weather + 0.05 * n(),
    humidity: 45 - 5 * Math.sin(day - 2) + 1.5 * drift - 3 * weather + 0.2 * n(),
    illuminance: Math.max(0, Math.sin(day - 1.75) * 720) + 25 + 8 * n(),
    pressure: 100150 + 650 * weather + 40 * Math.sin(wall / 7200) + 3 * n(),
    // People in the room during the day.
    co2: AIR ? Math.round(620 + 380 * Math.max(0, Math.sin(day - 2.2)) + 40 * drift + 8 * n()) : null,
    voc: AIR ? Math.round(110 + 90 * Math.max(0, Math.sin(day - 2.4)) + 20 * drift + 5 * n()) : null,
  };
}

function reading(ts: number, channel = 0): Reading {
  const v = values(ts, channel);
  return { ts, device: "TB4F2A91", channel, ...v, altitude: 44330.77 * (1 - Math.pow(v.pressure / 101325, 0.190263)) };
}

function history(from: number, to: number, maxPoints = 900): History {
  const ladder = [2e3, 5e3, 1e4, 3e4, 6e4, 12e4, 3e5, 6e5, 9e5, 18e5, 36e5, 72e5, 216e5, 432e5, 864e5];
  const bucket = ladder.find((b) => b >= (to - from) / maxPoints && b >= 1e4) ?? 864e5;
  // ?gap=<hours>: data only for the last <hours> hours (the data range claims 30 days, to test panning limits).
  const gap = params.has("gap") ? Number(params.get("gap")) * 3_600_000 : null;
  const first = gap === null ? Date.now() - 14 * DAY : Date.now() - gap;
  const empty = (): Series[] => CHANNELS.map(() => ({ avg: [], min: [], max: [] }));
  const h: History = {
    from,
    to,
    bucket_ms: bucket,
    t: [],
    channels: CHANNELS,
    temperature: empty(),
    humidity: empty(),
    illuminance: empty(),
    pressure: empty(),
    co2: empty(),
    voc: empty(),
  };
  for (let b = Math.floor(Math.max(from, first) / bucket) * bucket; b < Math.min(to, Date.now()); b += bucket) {
    h.t.push((b + bucket / 2) / 1000);
    CHANNELS.forEach((channel, c) => {
      const samples = [0, 0.33, 0.66].map((f) => values(b + f * bucket, channel));
      for (const m of METRICS) {
        const xs = samples.map((s) => s[m]).filter((x): x is number => x !== null);
        h[m][c].avg.push(xs.length ? xs.reduce((a, x) => a + x, 0) / xs.length : null);
        h[m][c].min.push(xs.length ? Math.min(...xs) : null);
        h[m][c].max.push(xs.length ? Math.max(...xs) : null);
      }
    });
  }
  return h;
}

const SOURCES: SensorSource[] = [
  {
    metric: "temperature",
    channel: 0,
    module: "climate_module_r2",
    sensor: "SHT30",
    address: 0x45,
  },
  {
    metric: "humidity",
    channel: 0,
    module: "climate_module_r2",
    sensor: "SHT30",
    address: 0x45,
  },
  {
    metric: "illuminance",
    channel: 0,
    module: "climate_module_r2",
    sensor: "OPT3001",
    address: 0x44,
  },
  {
    metric: "pressure",
    channel: 0,
    module: "climate_module_r2",
    sensor: "MPL3115A2",
    address: 0x60,
  },
  ...(AIR
    ? ([
        { metric: "co2", channel: 0, module: "co2_module", sensor: "LP8", address: 0x4d },
        { metric: "voc", channel: 0, module: "voc_lp_tag", sensor: "SGPC3", address: 0x58 },
      ] satisfies SensorSource[])
    : []),
  ...(CHANNELS.includes(1)
    ? ([
        {
          metric: "temperature",
          channel: 1,
          module: "humidity_tag",
          sensor: "SHT20",
          address: 0x40,
        },
        {
          metric: "humidity",
          channel: 1,
          module: "humidity_tag",
          sensor: "SHT20",
          address: 0x40,
        },
        {
          metric: "illuminance",
          channel: 1,
          module: "lux_meter_tag",
          sensor: "OPT3001",
          address: 0x44,
        },
        {
          metric: "pressure",
          channel: 1,
          module: "barometer_tag",
          sensor: "MPL3115A2",
          address: 0x60,
        },
      ] satisfies SensorSource[])
    : []),
];

const device: DeviceState = (() => {
  const info = {
    backend: "hid" as const,
    serial: "TB4F2A91",
    path: "IOService:/…/FT260",
    chip: "0260 rev 0100",
    bridges_connected: Number(params.get("bridges") ?? 1),
  };
  switch (params.get("state")) {
    case "not_found":
      return { status: "error", kind: "not_found", message: "No TOWER Bridge Module found" };
    case "no_module":
      return { status: "no_module", device: info };
    case "permission":
      return { status: "error", kind: "permission_denied", message: "/dev/hidraw3: Permission denied" };
    default:
      return { status: "ready", device: info, sources: SOURCES };
  }
})();

const settings: Settings = {
  version: 3,
  theme: (params.get("theme") as ThemePref) ?? "system",
  language: (params.get("lang") as LanguagePref) ?? "system",
  date_format: (params.get("date") as DateFormat) ?? "auto",
  time_format: (params.get("time") as TimeFormat) ?? "auto",
  decimal_separator: (params.get("decimal") as DecimalSeparator) ?? "auto",
  temperature_unit: "celsius",
  sample_interval_ms: 20_000,
  tray_metric: "temperature",
  retention_days: 90,
  sea_level_pressure_pa: 101325,
  i2c_channel: "auto",
  backend: "auto",
  alerts: {
    temperature: { enabled: true, min: 18, max: 24, hysteresis: 0.5, cooldown_ms: 900_000, notify_recovery: true },
    humidity: { enabled: true, min: 30, max: 60, hysteresis: 2, cooldown_ms: 900_000, notify_recovery: true },
    illuminance: { enabled: false, min: null, max: null, hysteresis: 10, cooldown_ms: 900_000, notify_recovery: true },
    pressure: { enabled: false, min: null, max: null, hysteresis: 100, cooldown_ms: 900_000, notify_recovery: true },
    co2: { enabled: true, min: null, max: 1200, hysteresis: 50, cooldown_ms: 900_000, notify_recovery: true },
    voc: { enabled: false, min: null, max: 500, hysteresis: 25, cooldown_ms: 900_000, notify_recovery: true },
  },
  notify_device_lost: true,
  mqtt: {
    enabled: mqttParam !== "off",
    host: mqttParam === "no_host" ? "" : "homeassistant.local",
    port: 1883,
    tls: false,
    username: "",
    password: "",
    client_id: "tower-bridge-4f2a91c0",
    topic_prefix: "tower",
    allow_control: true,
  },
  relay: { pulse_duration_ms: 2000 },
  charts: {
    temperature: { colors: ["orange", "teal"], show_min: true, show_avg: true, show_max: true, hidden_channels: [] },
    humidity: { colors: ["blue", "pink"], show_min: false, show_avg: true, show_max: false, hidden_channels: [] },
    illuminance: { colors: ["gold", "violet"], show_min: false, show_avg: false, show_max: true, hidden_channels: [] },
    pressure: { colors: ["violet", "green"], show_min: false, show_avg: true, show_max: false, hidden_channels: [] },
    co2: { colors: ["green", "pink"], show_min: false, show_avg: true, show_max: true, hidden_channels: [] },
    voc: { colors: ["teal", "orange"], show_min: false, show_avg: true, show_max: false, hidden_channels: [] },
  },
  api: {
    enabled: true,
    port: 8180,
    allow_lan: false,
    mcp: true,
    allow_control: false,
    token: "3f9a1c0e7b2d4a6f8e1c3b5d7f9a1c3e5b7d9f1a3c5e7b9d",
  },
};

const now = Date.now();
const latest = device.status === "ready" ? CHANNELS.map((c) => reading(now, c)) : [];
// ?relay=true|false|unknown shows a Relay Module.
const relayParam = params.get("relay");
let relay: RelayInfo | null =
  relayParam === null
    ? null
    : { channel: 0, address: 0x3b, state: relayParam === "unknown" ? null : relayParam === "true", pulse_until: null };

// ?nfc=url|empty|unformatted shows an NFC Tag on channel 0.
const nfcParam = params.get("nfc");
let nfc: NfcInfo | null =
  nfcParam === null
    ? null
    : {
        channel: 0,
        address: 0x55,
        part: "NT3H2111",
        url: nfcParam === "url" ? "https://www.hardwario.com" : null,
        formatted: nfcParam !== "unformatted",
      };

// ?gps=fix|nofix shows a GPS Module on channel 0.
const gpsParam = params.get("gps");
const gps: GpsInfo | null =
  gpsParam === null
    ? null
    : gpsParam === "fix"
      ? {
          channel: 0,
          fix: true,
          satellites: 9,
          latitude: 50.771168,
          longitude: 15.056328,
          altitude: 374.5,
          accuracy: 2.4,
          time: Date.now(),
        }
      : {
          channel: 0,
          fix: false,
          satellites: 0,
          latitude: null,
          longitude: null,
          altitude: null,
          accuracy: null,
          time: null,
        };

const snapshot: Snapshot = {
  device,
  relay,
  nfc,
  gps,
  latest,
  settings,
  open_at_login: true,
  mqtt:
    mqttParam === "off"
      ? { status: "disabled" }
      : mqttParam === "no_host"
        ? { status: "error", message: "no_host" }
        : mqttParam === "connecting"
          ? {
              status: "connecting",
              address: "homeassistant.local:1883",
              error: "connection refused",
            }
          : { status: "connected", address: "homeassistant.local:1883" },
  api: { status: "running", address: "127.0.0.1:8180", url: "http://127.0.0.1:8180" },
  active_alerts:
    latest[0] && latest[0].temperature! > 24
      ? [
          {
            metric: "temperature",
            channel: 0,
            bound: "high",
            since: now - 600_000,
            threshold: 24,
            value: latest[0].temperature!,
          },
        ]
      : [],
  app: { version: "0.1.0", data_dir: "~/Library/Application Support/com.hardwario.tower-bridge", portable: false, simulated: false, os: "macos", system_language: "en", exe_path: "/Applications/TOWER Bridge.app/Contents/MacOS/tower-bridge" },
};

mockIPC(
  (cmd, args) => {
    const a = (args ?? {}) as Record<string, unknown>;
    switch (cmd) {
      case "get_snapshot":
        return snapshot;
      case "get_data_range":
        return { first: Date.now() - (params.has("gap") ? 30 : 14) * 86_400_000, last: Date.now() };
      case "get_history":
        return history(a.from as number, a.to as number, a.max_points as number | undefined);
      case "get_alert_log":
        return [
          {
            ts: now - 3_600_000,
            metric: "temperature",
            channel: 0,
            bound: "high",
            kind: "triggered",
            value: 24.6,
            threshold: 24,
          },
          {
            ts: now - 2_400_000,
            metric: "temperature",
            channel: 0,
            bound: "high",
            kind: "recovered",
            value: 23.4,
            threshold: 24,
          },
          {
            ts: now - 30 * 3_600_000,
            metric: "humidity",
            channel: 0,
            bound: "low",
            kind: "triggered",
            value: 29.1,
            threshold: 30,
          },
        ];
      case "save_settings":
        return a.settings;
      case "set_open_at_login":
        return a.enabled;
      case "open_link":
        console.info("open link", a.target);
        return null;
      case "identify_device":
        return new Promise((resolve) => setTimeout(() => resolve(null), 800));
      case "set_relay":
        return new Promise((resolve) =>
          setTimeout(() => resolve((relay = { channel: 0, address: 0x3b, state: a.state as boolean, pulse_until: null })), 300),
        );
      case "pulse_relay": {
        const ms = (a.duration_ms as number | null) ?? settings.relay.pulse_duration_ms;
        const to = (a.state as boolean | null) ?? !(relay?.state ?? true);
        relay = { channel: 0, address: 0x3b, state: to, pulse_until: Date.now() + ms };
        setTimeout(() => {
          relay = { channel: 0, address: 0x3b, state: !to, pulse_until: null };
          void emit("relay-state", relay);
        }, ms);
        return new Promise((resolve) => setTimeout(() => resolve(relay), 300));
      }
      case "open_map":
        return null;
      case "write_nfc_url":
        return new Promise((resolve, reject) =>
          setTimeout(() => {
            if (!nfc) return reject({ code: "no_nfc", message: "TOWER NFC Tag not found" });
            nfc = { ...nfc, url: a.url as string };
            void emit("nfc-state", nfc);
            resolve(nfc);
          }, 500),
        );
      case "rescan_device":
        return new Promise((resolve) => setTimeout(() => resolve({ status: "found", sources: SOURCES }), 600));
      default:
        return null;
    }
  },
  { shouldMockEvents: true },
);

await import("./main");

if (latest.length) {
  setInterval(() => {
    const ts = Date.now();
    for (const c of CHANNELS) void emit("reading", reading(ts, c));
  }, 10_000);
}

const action = params.get("action");
if (action) {
  setTimeout(() => {
    const label = action === "identify" ? 0 : 1;
    document.querySelectorAll<HTMLButtonElement>(".device .actions button")[label]?.click();
  }, 500);
}

// ?drag=0.3-0.6: drag-select that horizontal fraction of the first chart (zoom).
// ?hover=0.7: move the pointer to that fraction of the first chart.
function chartPoint(fx: number, fy = 0.5) {
  const over = document.querySelector<HTMLElement>(".u-over");
  if (!over) return null;
  const r = over.getBoundingClientRect();
  return { over, x: r.left + r.width * fx, y: r.top + r.height * fy };
}
const drag = params.get("drag");
if (drag) {
  setTimeout(() => {
    const [a, b] = drag.split("-").map(Number);
    const p0 = chartPoint(a);
    const p1 = chartPoint(b);
    if (!p0 || !p1) return;
    // Shift+drag selects a range. uPlot tracks drags on the plot and ignores moves without movement.
    const ev = (type: string, p: { x: number; y: number }) =>
      new MouseEvent(type, { bubbles: true, clientX: p.x, clientY: p.y, button: 0, buttons: 1, movementX: 5, shiftKey: true });
    p0.over.dispatchEvent(ev("mousedown", p0));
    p0.over.dispatchEvent(ev("mousemove", p1));
    document.dispatchEvent(ev("mouseup", p1));
  }, 1200);
}
// ?pan=<x0>-<x1>: plain drag (pans the time axis).
const pan = params.get("pan");
if (pan) {
  setTimeout(() => {
    const [range, times] = pan.split("x");
    const [a, b] = range.split("-").map(Number);
    const p0 = chartPoint(a);
    const p1 = chartPoint(b);
    if (!p0 || !p1) return;
    const ev = (type: string, p: { x: number; y: number }) =>
      new MouseEvent(type, { bubbles: true, clientX: p.x, clientY: p.y, button: 0, buttons: 1 });
    // Repeated drags, with time for the panned-to data to load in between.
    for (let i = 0; i < Number(times ?? 1); i++) {
      setTimeout(() => {
        p0.over.dispatchEvent(ev("mousedown", p0));
        document.dispatchEvent(ev("mousemove", p1));
        document.dispatchEvent(ev("mouseup", p1));
      }, i * 400);
    }
  }, 1800);
}

// ?reorder=<panel>:<target>[:hold]: drag a panel by its handle onto another one (hold: do not release).
const reorder = params.get("reorder");
if (reorder) {
  setTimeout(() => {
    const [from, to, hold] = reorder.split(":");
    const grip = document.querySelector<HTMLElement>(`[data-panel="${from}"] .grip`);
    const target = document.querySelector<HTMLElement>(`[data-panel="${to}"]`)?.getBoundingClientRect();
    if (!grip || !target) return;
    const g = grip.getBoundingClientRect();
    const at = (x: number, y: number) => ({ bubbles: true, clientX: x, clientY: y, button: 0, pointerId: 1 });
    grip.dispatchEvent(new PointerEvent("pointerdown", at(g.x + 5, g.y + 5)));
    const [tx, ty] = [target.x + target.width / 2, target.y + target.height / 2];
    for (let i = 1; i <= 10; i++) {
      const [x, y] = [g.x + ((tx - g.x) * i) / 10, g.y + ((ty - g.y) * i) / 10];
      setTimeout(() => window.dispatchEvent(new PointerEvent("pointermove", at(x, y))), i * 30);
    }
    if (!hold) setTimeout(() => window.dispatchEvent(new PointerEvent("pointerup", at(tx, ty))), 400);
  }, 1500);
}

// ?wheel=<x>:<deltaY>[:pan]: wheel zoom (or sideways pan) at a relative x position.
const wheel = params.get("wheel");
if (wheel) {
  setTimeout(() => {
    const [x, delta, mode] = wheel.split(":");
    const p = chartPoint(Number(x));
    const d = Number(delta);
    p?.over.dispatchEvent(
      new WheelEvent("wheel", {
        bubbles: true,
        cancelable: true,
        clientX: p.x,
        clientY: p.y,
        ctrlKey: false,
        deltaX: mode === "pan" ? d : 0,
        deltaY: mode === "pan" ? 0 : d,
      }),
    );
  }, 1600);
}
const hover = params.get("hover");
if (hover) {
  setTimeout(() => {
    const p = chartPoint(Number(hover));
    p?.over.dispatchEvent(new MouseEvent("mouseenter", { bubbles: true, clientX: p.x, clientY: p.y }));
    p?.over.dispatchEvent(new MouseEvent("mousemove", { bubbles: true, clientX: p.x, clientY: p.y }));
  }, 2000);
}

// ?focus=<CSS selector>: focus an element (e.g. to open a hover popover).
const focusSel = params.get("focus");
if (focusSel) setTimeout(() => document.querySelector<HTMLElement>(focusSel)?.focus(), 1500);

// ?click=<CSS selector>: click an element once the view is shown.
const click = params.get("click");
if (click) setTimeout(() => document.querySelector<HTMLElement>(click)?.click(), 800);

const tab = params.get("tab");
if (tab) {
  setTimeout(() => {
    document.querySelector<HTMLButtonElement>(`nav button[data-tab="${tab}"]`)?.click();
  }, 300);
}
