import { api, on } from "./api";
import { i18n, resolveLang } from "./i18n/index.svelte";
import { theme } from "./theme.svelte";
import type {
  ActiveAlert,
  AlertEvent,
  ApiState,
  AppInfo,
  DeviceState,
  MqttState,
  Reading,
  RelayInfo,
  SensorSource,
  Settings,
} from "./types";

const RECENT_MS = 3_600_000;

/** App state mirrored from the backend and kept current by events. */
class Live {
  ready = $state(false);
  device = $state<DeviceState>({ status: "searching" });
  relay = $state<RelayInfo | null>(null);
  /** The latest reading of each I²C channel, in channel order. */
  latest = $state<Reading[]>([]);
  settings = $state<Settings | null>(null);
  openAtLogin = $state(false);
  mqtt = $state<MqttState>({ status: "disabled" });
  api = $state<ApiState>({ status: "disabled" });
  activeAlerts = $state<ActiveAlert[]>([]);
  app = $state<AppInfo | null>(null);
  /** Readings received during this session (last hour), for sparklines. */
  recent = $state<Reading[]>([]);
  /** Alert events received during this session, newest first. */
  alertEvents = $state<AlertEvent[]>([]);
  /** Incremented on every reading so views can refresh. */
  tick = $state(0);

  async init(): Promise<void> {
    // State that changed by event while the snapshot was on its way is newer
    // than the snapshot (these events come only on change).
    const fresh = new Set<string>();
    await Promise.all([
      on("reading", (r) => this.pushReading(r)),
      on("device-state", (d) => {
        fresh.add("device");
        this.setDevice(d);
      }),
      on("relay-state", (r) => {
        fresh.add("relay");
        this.relay = r;
      }),
      on("settings-changed", (s) => this.setSettings(s)),
      on("open-at-login-changed", (v) => (this.openAtLogin = v)),
      on("alerts-active", (a) => {
        fresh.add("alerts");
        this.activeAlerts = a;
      }),
      on("alert", (e) => (this.alertEvents = [e, ...this.alertEvents].slice(0, 100))),
      on("mqtt-state", (m) => (this.mqtt = m)),
      on("api-state", (a) => (this.api = a)),
    ]);
    const s = await api.snapshot();
    if (!fresh.has("device")) this.setDevice(s.device);
    if (!fresh.has("relay")) this.relay = s.relay;
    this.app = s.app;
    this.setSettings(s.settings);
    this.openAtLogin = s.open_at_login;
    this.mqtt = s.mqtt;
    this.api = s.api;
    if (!fresh.has("alerts")) this.activeAlerts = s.active_alerts;
    for (const r of s.latest ?? []) this.pushReading(r);
    this.ready = true;
  }

  /** Store settings and apply theme and language right away. */
  private setSettings(s: Settings) {
    this.settings = s;
    theme.apply(s.theme);
    i18n.set(resolveLang(s.language, this.app?.system_language));
    i18n.dateFormat = s.date_format ?? "auto";
    i18n.timeFormat = s.time_format ?? "auto";
    i18n.decimal = s.decimal_separator ?? "auto";
  }

  /** A channel without sensors has no current values any more (as in the backend). */
  private setDevice(d: DeviceState) {
    this.device = d;
    if (d.status === "ready") this.latest = pruneLatest(this.latest, d.sources);
  }

  private pushReading(r: Reading) {
    const known = this.latest.find((x) => x.channel === r.channel);
    if (known && known.ts > r.ts) return; // an older reading (the snapshot's) arrived late
    this.latest = [...this.latest.filter((x) => x.channel !== r.channel), r].sort((a, b) => a.channel - b.channel);
    const cutoff = r.ts - RECENT_MS;
    const recent = this.recent.filter((x) => x.ts >= cutoff && !(x.ts === r.ts && x.channel === r.channel));
    recent.push(r);
    this.recent = recent;
    this.tick++;
  }

  /** Persist settings (optimistic update). */
  async save(mutate: (s: Settings) => void): Promise<void> {
    if (!this.settings) return;
    const next = structuredClone($state.snapshot(this.settings)) as Settings;
    mutate(next);
    this.setSettings(next);
    this.setSettings(await api.saveSettings(next));
  }

  /** Time of the newest reading on any channel. */
  get newestTs(): number | null {
    return this.latest.reduce<number | null>((m, r) => (m === null || r.ts > m ? r.ts : m), null);
  }

  /** Active alerts of a metric, on any channel. */
  alertsFor(metric: string): ActiveAlert[] {
    return this.activeAlerts.filter((a) => a.metric === metric);
  }

  alertFor(metric: string): ActiveAlert | undefined {
    return this.activeAlerts.find((a) => a.metric === metric);
  }
}

/** The readings of channels that still have sensors. */
export function pruneLatest(latest: Reading[], sources: SensorSource[]): Reading[] {
  const channels = new Set(sources.map((s) => s.channel));
  return latest.filter((r) => channels.has(r.channel));
}

export const live = new Live();
