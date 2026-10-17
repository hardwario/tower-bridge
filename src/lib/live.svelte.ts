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
  Settings,
} from "./types";

const RECENT_MS = 3_600_000;

/** App state mirrored from the backend and kept current by events. */
class Live {
  ready = $state(false);
  device = $state<DeviceState>({ status: "searching" });
  relay = $state<RelayInfo | null>(null);
  latest = $state<Reading | null>(null);
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
    await Promise.all([
      on("reading", (r) => this.pushReading(r)),
      on("device-state", (d) => (this.device = d)),
      on("relay-state", (r) => (this.relay = r)),
      on("settings-changed", (s) => this.setSettings(s)),
      on("open-at-login-changed", (v) => (this.openAtLogin = v)),
      on("alerts-active", (a) => (this.activeAlerts = a)),
      on("alert", (e) => (this.alertEvents = [e, ...this.alertEvents].slice(0, 100))),
      on("mqtt-state", (m) => (this.mqtt = m)),
      on("api-state", (a) => (this.api = a)),
    ]);
    const s = await api.snapshot();
    this.device = s.device;
    this.relay = s.relay;
    this.latest = s.latest;
    this.app = s.app;
    this.setSettings(s.settings);
    this.openAtLogin = s.open_at_login;
    this.mqtt = s.mqtt;
    this.api = s.api;
    this.activeAlerts = s.active_alerts;
    this.app = s.app;
    if (s.latest) this.pushReading(s.latest);
    this.ready = true;
  }

  /** Store settings and apply theme and language right away. */
  private setSettings(s: Settings) {
    this.settings = s;
    theme.apply(s.theme);
    i18n.set(resolveLang(s.language, this.app?.system_language));
    i18n.dateFormat = s.date_format ?? "auto";
    i18n.timeFormat = s.time_format ?? "auto";
  }

  private pushReading(r: Reading) {
    this.latest = r;
    const cutoff = r.ts - RECENT_MS;
    const recent = this.recent.filter((x) => x.ts >= cutoff && x.ts !== r.ts);
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

  alertFor(metric: string): ActiveAlert | undefined {
    return this.activeAlerts.find((a) => a.metric === metric);
  }
}

export const live = new Live();
