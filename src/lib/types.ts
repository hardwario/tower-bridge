// Mirrors the serde types of the Rust backend.

import type { Lang, LanguagePref } from "./i18n/index.svelte";
import type { ThemePref } from "./theme.svelte";

export type Metric = "temperature" | "humidity" | "illuminance" | "pressure";
export const METRICS: Metric[] = ["temperature", "humidity", "illuminance", "pressure"];

export interface Values {
  temperature: number | null;
  humidity: number | null;
  illuminance: number | null;
  pressure: number | null;
}

export interface Reading extends Values {
  ts: number;
  device: string;
  altitude: number | null;
}

export interface DeviceInfo {
  backend: "hid" | "i2c_dev";
  serial: string;
  path: string;
  chip: string | null;
}

export type DeviceErrorKind = "not_found" | "permission_denied" | "i2c_dev_not_loaded" | "busy" | "failed";

export type DeviceState =
  | { status: "searching" }
  | { status: "error"; kind: DeviceErrorKind; message: string }
  | { status: "no_module"; device: DeviceInfo }
  | { status: "ready"; device: DeviceInfo; revision: "R1" | "R2"; channel: number; sources: SensorSource[] };

/** The sensor behind one value and its I²C address. */
export interface SensorSource {
  metric: Metric;
  sensor: string;
  address: number;
}

export interface AlertRule {
  enabled: boolean;
  min: number | null;
  max: number | null;
  hysteresis: number;
  cooldown_s: number;
  notify_recovery: boolean;
}

export type AlertRules = Record<Metric, AlertRule>;

export type TemperatureUnit = "celsius" | "fahrenheit";
export type ChannelMode = "auto" | "channel0" | "channel1";
/** "auto" follows the language; the others are 2026-10-17, 17.10.2026, 17/10/2026, 10/17/2026. */
export type DateFormat = "auto" | "iso" | "dmy" | "dmy_slash" | "mdy";
export type TimeFormat = "auto" | "h24" | "h12";
export const DATE_FORMATS: DateFormat[] = ["auto", "iso", "dmy", "dmy_slash", "mdy"];
export const TIME_FORMATS: TimeFormat[] = ["auto", "h24", "h12"];

export interface MqttSettings {
  enabled: boolean;
  port: number;
  allow_lan: boolean;
  topic_prefix: string;
  username: string;
  password: string;
  /** Accept relay commands on <prefix>/relay/set and <prefix>/relay/pulse. */
  allow_control: boolean;
}

/** Line colours of the dashboard palette (see palette.ts). */
export type ChartColor = "blue" | "green" | "pink" | "gold" | "violet" | "teal" | "orange" | "graphite";

/** How one metric's charts look. */
export interface ChartStyle {
  color: ChartColor;
  show_min: boolean;
  show_avg: boolean;
  show_max: boolean;
}

export interface RelaySettings {
  /** Length of a timed pulse. */
  pulse_duration_ms: number;
}

export interface ApiSettings {
  enabled: boolean;
  port: number;
  allow_lan: boolean;
  mcp: boolean;
  allow_control: boolean;
  token: string;
}

export interface Settings {
  version: number;
  theme: ThemePref;
  language: LanguagePref;
  date_format: DateFormat;
  time_format: TimeFormat;
  temperature_unit: TemperatureUnit;
  sample_interval_s: number;
  tray_metric: Metric | null;
  retention_days: number;
  sea_level_pressure_pa: number;
  i2c_channel: ChannelMode;
  backend: "auto" | "hid" | "i2c_dev";
  alerts: AlertRules;
  notify_device_lost: boolean;
  mqtt: MqttSettings;
  api: ApiSettings;
  relay: RelaySettings;
  charts: Record<Metric, ChartStyle>;
}

export type MqttState =
  | { status: "disabled" }
  | { status: "running"; address: string }
  | { status: "error"; message: string }
  | { status: "restart_required"; reason: string };

export type ApiState =
  | { status: "disabled" }
  | { status: "running"; address: string; url: string }
  | { status: "error"; port: number; message: string };

export type Bound = "low" | "high";

export interface ActiveAlert {
  metric: Metric;
  bound: Bound;
  since: number;
  threshold: number;
  value: number;
}

export interface AlertEvent {
  ts: number;
  metric: Metric;
  bound: Bound;
  kind: "triggered" | "recovered";
  value: number;
  threshold: number;
  notify: boolean;
}

export interface AlertLogEntry {
  ts: number;
  metric: Metric;
  bound: Bound;
  kind: "triggered" | "recovered";
  value: number;
  threshold: number;
}

export interface Series {
  avg: (number | null)[];
  min: (number | null)[];
  max: (number | null)[];
}

export interface History {
  from: number;
  to: number;
  bucket_ms: number;
  t: number[];
  temperature: Series;
  humidity: Series;
  illuminance: Series;
  pressure: Series;
}

export interface AppInfo {
  version: string;
  data_dir: string;
  portable: boolean;
  simulated: boolean;
  os: string;
  system_language: Lang;
  exe_path: string | null;
}

/** Detected TOWER Relay Module. */
export interface RelayInfo {
  channel: number;
  address: number;
  /** true (left), false (right), or null while unknown: a latching relay cannot be read back. */
  state: boolean | null;
  /** End of a running timed pulse (Unix ms); the relay then switches back. */
  pulse_until: number | null;
}

export interface Snapshot {
  device: DeviceState;
  relay: RelayInfo | null;
  latest: Reading | null;
  settings: Settings;
  open_at_login: boolean;
  mqtt: MqttState;
  api: ApiState;
  active_alerts: ActiveAlert[];
  app: AppInfo;
}

export type RescanOutcome = { status: "found"; revision: "R1" | "R2"; channel: number } | { status: "no_module" };

export interface ExportResult {
  path: string;
  rows: number;
}
