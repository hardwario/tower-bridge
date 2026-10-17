// Development preview: runs the dashboard in a plain browser with a mocked
// backend (no Tauri, no hardware). Open http://localhost:1420/preview.html
// Query parameters: ?tab=history|alerts|settings&state=ready|notFound|noModule|permission&action=identify|rescan
//                   &theme=system|light|dark&lang=system|en|cs|sk|de|pl&date=auto|iso|dmy|dmy_slash|mdy&time=auto|h24|h12

import { mockIPC } from "@tauri-apps/api/mocks";
import { emit } from "@tauri-apps/api/event";
import type { LanguagePref } from "./lib/i18n/index.svelte";
import type { ThemePref } from "./lib/theme.svelte";
import type {
  DateFormat,
  DeviceState,
  History,
  Reading,
  RelayInfo,
  Series,
  Settings,
  Snapshot,
  TimeFormat,
} from "./lib/types";

const params = new URLSearchParams(location.search);
const DAY = 86_400_000;

function values(ts: number) {
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
  };
}

function reading(ts: number): Reading {
  const v = values(ts);
  return { ts, device: "TB4F2A91", ...v, altitude: 44330.77 * (1 - Math.pow(v.pressure / 101325, 0.190263)) };
}

function history(from: number, to: number, maxPoints = 900): History {
  const ladder = [2e3, 5e3, 1e4, 3e4, 6e4, 12e4, 3e5, 6e5, 9e5, 18e5, 36e5, 72e5, 216e5, 432e5, 864e5];
  const bucket = ladder.find((b) => b >= (to - from) / maxPoints && b >= 1e4) ?? 864e5;
  // ?gap=<hours>: data only for the last <hours> hours plus one stray reading 30 days ago.
  const gap = params.has("gap") ? Number(params.get("gap")) * 3_600_000 : null;
  const first = gap === null ? Date.now() - 14 * DAY : Date.now() - gap;
  const empty = (): Series => ({ avg: [], min: [], max: [] });
  const h: History = { from, to, bucket_ms: bucket, t: [], temperature: empty(), humidity: empty(), illuminance: empty(), pressure: empty() };
  for (let b = Math.floor(Math.max(from, first) / bucket) * bucket; b < Math.min(to, Date.now()); b += bucket) {
    h.t.push((b + bucket / 2) / 1000);
    const samples = [0, 0.33, 0.66].map((f) => values(b + f * bucket));
    for (const m of ["temperature", "humidity", "illuminance", "pressure"] as const) {
      const xs = samples.map((s) => s[m]);
      h[m].avg.push(xs.reduce((a, c) => a + c, 0) / xs.length);
      h[m].min.push(Math.min(...xs));
      h[m].max.push(Math.max(...xs));
    }
  }
  return h;
}

const device: DeviceState = (() => {
  const info = { backend: "hid" as const, serial: "TB4F2A91", path: "IOService:/…/FT260", chip: "0260 rev 0100" };
  switch (params.get("state")) {
    case "not_found":
      return { status: "error", kind: "not_found", message: "No TOWER Bridge Module found" };
    case "no_module":
      return { status: "no_module", device: info };
    case "permission":
      return { status: "error", kind: "permission_denied", message: "/dev/hidraw3: Permission denied" };
    default:
      return {
        status: "ready",
        device: info,
        revision: "R2",
        channel: 0,
        sources: [
          { metric: "temperature", sensor: "SHT30", address: 0x45 },
          { metric: "humidity", sensor: "SHT30", address: 0x45 },
          { metric: "illuminance", sensor: "OPT3001", address: 0x44 },
          { metric: "pressure", sensor: "MPL3115A2", address: 0x60 },
        ],
      };
  }
})();

const settings: Settings = {
  version: 1,
  theme: (params.get("theme") as ThemePref) ?? "system",
  language: (params.get("lang") as LanguagePref) ?? "system",
  date_format: (params.get("date") as DateFormat) ?? "auto",
  time_format: (params.get("time") as TimeFormat) ?? "auto",
  temperature_unit: "celsius",
  sample_interval_s: 10,
  tray_metric: "temperature",
  retention_days: 90,
  sea_level_pressure_pa: 101325,
  i2c_channel: "auto",
  backend: "auto",
  alerts: {
    temperature: { enabled: true, min: 18, max: 24, hysteresis: 0.5, cooldown_s: 900, notify_recovery: true },
    humidity: { enabled: true, min: 30, max: 60, hysteresis: 2, cooldown_s: 900, notify_recovery: true },
    illuminance: { enabled: false, min: null, max: null, hysteresis: 10, cooldown_s: 900, notify_recovery: true },
    pressure: { enabled: false, min: null, max: null, hysteresis: 100, cooldown_s: 900, notify_recovery: true },
  },
  notify_device_lost: true,
  mqtt: { enabled: true, port: 1883, allow_lan: false, topic_prefix: "tower", username: "", password: "", allow_control: true },
  relay: { pulse_duration_ms: 2000 },
  charts: {
    temperature: { color: "orange", show_min: true, show_avg: true, show_max: true },
    humidity: { color: "blue", show_min: false, show_avg: true, show_max: false },
    illuminance: { color: "gold", show_min: false, show_avg: false, show_max: true },
    pressure: { color: "violet", show_min: false, show_avg: true, show_max: false },
  },
  api: { enabled: true, port: 8180, allow_lan: false, mcp: true, allow_control: false, token: "3f9a1c0e7b2d4a6f8e1c3b5d7f9a1c3e5b7d9f1a3c5e7b9d" },
};

const now = Date.now();
const latest = device.status === "ready" ? reading(now) : null;
// ?relay=true|false|unknown shows a Relay Module.
const relayParam = params.get("relay");
let relay: RelayInfo | null =
  relayParam === null
    ? null
    : { channel: 0, address: 0x3b, state: relayParam === "unknown" ? null : relayParam === "true", pulse_until: null };

const snapshot: Snapshot = {
  device,
  relay,
  latest,
  settings,
  open_at_login: true,
  mqtt: { status: "running", address: "127.0.0.1:1883" },
  api: { status: "running", address: "127.0.0.1:8180", url: "http://127.0.0.1:8180" },
  active_alerts:
    latest && latest.temperature! > 24
      ? [{ metric: "temperature", bound: "high", since: now - 600_000, threshold: 24, value: latest.temperature! }]
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
          { ts: now - 3_600_000, metric: "temperature", bound: "high", kind: "triggered", value: 24.6, threshold: 24 },
          { ts: now - 2_400_000, metric: "temperature", bound: "high", kind: "recovered", value: 23.4, threshold: 24 },
          { ts: now - 30 * 3_600_000, metric: "humidity", bound: "low", kind: "triggered", value: 29.1, threshold: 30 },
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
      case "rescan_device":
        return new Promise((resolve) =>
          setTimeout(() => resolve({ status: "found", revision: "R1", channel: 0 }), 600),
        );
      default:
        return null;
    }
  },
  { shouldMockEvents: true },
);

await import("./main");

if (latest) {
  setInterval(() => void emit("reading", reading(Date.now())), 10_000);
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
