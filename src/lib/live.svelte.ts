import { api, errorMessage, on } from "./api";
import { i18n, resolveLang } from "./i18n/index.svelte";
import { rememberSources } from "./source";
import { theme } from "./theme.svelte";
import type {
  ActiveAlert,
  AlertEvent,
  ApiState,
  AppInfo,
  DeviceState,
  GpsInfo,
  MqttState,
  NfcInfo,
  Reading,
  RelayInfo,
  SensorSource,
  Settings,
} from "./types";

/** App state mirrored from the backend and kept current by events. */
class Live {
  ready = $state(false);
  device = $state<DeviceState>({ status: "searching" });
  relay = $state<RelayInfo | null>(null);
  nfc = $state<NfcInfo | null>(null);
  gps = $state<GpsInfo | null>(null);
  /** The latest reading of each I²C channel, in channel order. */
  latest = $state<Reading[]>([]);
  settings = $state<Settings | null>(null);
  openAtLogin = $state(false);
  mqtt = $state<MqttState>({ status: "disabled" });
  api = $state<ApiState>({ status: "disabled" });
  activeAlerts = $state<ActiveAlert[]>([]);
  app = $state<AppInfo | null>(null);
  /** Why the last settings save failed, until the next one works. */
  saveError = $state<string | null>(null);
  /** Saves sent and not answered yet; their events and replies are older
      than what the page shows. */
  private pending = 0;
  private saving: Promise<void> = Promise.resolve();
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
      on("nfc-state", (n) => {
        fresh.add("nfc");
        this.nfc = n;
      }),
      on("gps-state", (g) => {
        fresh.add("gps");
        this.gps = g;
      }),
      on("settings-changed", (s) => {
        fresh.add("settings");
        // An echo of a save still under way would undo later changes.
        if (this.pending === 0) this.setSettings(s);
      }),
      on("open-at-login-changed", (v) => {
        fresh.add("login");
        this.openAtLogin = v;
      }),
      on("alerts-active", (a) => {
        fresh.add("alerts");
        this.activeAlerts = a;
      }),
      on("alert", (e) => (this.alertEvents = [e, ...this.alertEvents].slice(0, 100))),
      on("mqtt-state", (m) => {
        fresh.add("mqtt");
        this.mqtt = m;
      }),
      on("api-state", (a) => {
        fresh.add("api");
        this.api = a;
      }),
    ]);
    const s = await api.snapshot();
    if (!fresh.has("device")) this.setDevice(s.device);
    if (!fresh.has("relay")) this.relay = s.relay;
    if (!fresh.has("nfc")) this.nfc = s.nfc ?? null;
    if (!fresh.has("gps")) this.gps = s.gps ?? null;
    this.app = s.app;
    if (!fresh.has("settings") || !this.settings) this.setSettings(s.settings);
    if (!fresh.has("login")) this.openAtLogin = s.open_at_login;
    if (!fresh.has("mqtt")) this.mqtt = s.mqtt;
    if (!fresh.has("api")) this.api = s.api;
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
    if (d.status === "ready") {
      this.latest = pruneLatest(this.latest, d.sources);
      rememberSources(d.sources);
    }
  }

  private pushReading(r: Reading) {
    const known = this.latest.find((x) => x.channel === r.channel);
    if (known && known.ts > r.ts) return; // an older reading (the snapshot's) arrived late
    this.latest = [...this.latest.filter((x) => x.channel !== r.channel), r].sort((a, b) => a.channel - b.channel);
    this.tick++;
  }

  /**
   * Persist settings: shown at once, saved one after another. Each save
   * sends what the page shows then, so quick changes in a row all arrive;
   * only the reply to the last one replaces the page's settings. A failed
   * save puts back what the backend has and says why.
   */
  async save(mutate: (s: Settings) => void): Promise<void> {
    if (!this.settings) return;
    const next = structuredClone($state.snapshot(this.settings)) as Settings;
    mutate(next);
    this.setSettings(next);
    this.pending++;
    const run = this.saving.then(async () => {
      try {
        const saved = await api.saveSettings(structuredClone($state.snapshot(this.settings)) as Settings);
        this.saveError = null;
        if (this.pending === 1) this.setSettings(saved);
      } catch (e) {
        this.saveError = errorMessage(e);
        try {
          this.setSettings((await api.snapshot()).settings);
        } catch {
          // The backend is gone; the page shows the error.
        }
      } finally {
        this.pending--;
      }
    });
    this.saving = run;
    return run;
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
