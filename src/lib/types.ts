// Mirrors the serde types of the Rust backend.

import type { Lang, LanguagePref } from "./i18n/index.svelte";
import type { ThemePref } from "./theme.svelte";

export type Metric = "temperature" | "humidity" | "illuminance" | "pressure" | "co2" | "voc";
export const METRICS: Metric[] = ["temperature", "humidity", "illuminance", "pressure", "co2", "voc"];
/** Shown only where a sensor measures them (the others always have a panel). */
export const OPTIONAL_METRICS: Metric[] = ["co2", "voc"];

export interface Values {
  temperature: number | null;
  humidity: number | null;
  illuminance: number | null;
  pressure: number | null;
  /** ppm */
  co2: number | null;
  /** ppb TVOC */
  voc: number | null;
}

/** The values measured on one I²C channel in one cycle. */
export interface Reading extends Values {
  ts: number;
  device: string;
  /** I²C switch channel of the sensors. */
  channel: number;
  altitude: number | null;
}

export interface DeviceInfo {
  backend: "hid" | "i2c_dev";
  serial: string;
  path: string;
  chip: string | null;
  /** TOWER Bridge Modules plugged in, this one included; only this one is read. */
  bridges_connected: number;
}

export type DeviceErrorKind = "not_found" | "permission_denied" | "i2c_dev_not_loaded" | "busy" | "failed";

export type DeviceState =
  | { status: "searching" }
  | { status: "error"; kind: DeviceErrorKind; message: string }
  | { status: "no_module"; device: DeviceInfo }
  | { status: "ready"; device: DeviceInfo; sources: SensorSource[] };

/** A TOWER module or tag that carries sensors. */
export type ModuleKind =
  | "climate_module_r1"
  | "climate_module_r2"
  | "humidity_tag"
  | "temperature_tag"
  | "lux_meter_tag"
  | "barometer_tag"
  | "co2_module"
  | "voc_lp_tag";

/** The sensor behind one value: I²C channel, module, part and address. */
export interface SensorSource {
  metric: Metric;
  channel: number;
  module: ModuleKind;
  sensor: string;
  address: number;
}

export interface AlertRule {
  enabled: boolean;
  min: number | null;
  max: number | null;
  hysteresis: number;
  cooldown_ms: number;
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
/** "auto" follows the language (a comma in cs, sk, de, pl; a point in en). */
export type DecimalSeparator = "auto" | "point" | "comma";
export const DECIMAL_SEPARATORS: DecimalSeparator[] = ["auto", "point", "comma"];

/** The MQTT broker the app publishes to, as a client. */
export interface MqttSettings {
  enabled: boolean;
  /** Empty until the user enters one. */
  host: string;
  port: number;
  tls: boolean;
  username: string;
  password: string;
  client_id: string;
  topic_prefix: string;
  /** Accept relay commands on <prefix>/relay/<channel>/set and .../pulse. */
  allow_control: boolean;
}

/** Line colours of the dashboard palette (see palette.ts). */
export type ChartColor = "blue" | "green" | "pink" | "gold" | "violet" | "teal" | "orange" | "graphite";

/** How one metric's charts look. */
export interface ChartStyle {
  /** Line colour of each I²C channel (index = channel). */
  colors: ChartColor[];
  show_min: boolean;
  show_avg: boolean;
  show_max: boolean;
  /** Channels whose lines the user hid; never all of them. */
  hidden_channels: number[];
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
  decimal_separator: DecimalSeparator;
  temperature_unit: TemperatureUnit;
  sample_interval_ms: number;
  tray_metric: Metric | null;
  retention_days: number;
  sea_level_pressure_pa: number;
  i2c_channel: ChannelMode;
  backend: "auto" | "hid" | "i2c_dev";
  alerts: AlertRules;
  notify_device_lost: boolean;
  /** Ask GitHub once a day whether a newer version is out. */
  check_for_updates: boolean;
  mqtt: MqttSettings;
  api: ApiSettings;
  relay: RelaySettings;
  charts: Record<Metric, ChartStyle>;
}

export type MqttState =
  | { status: "disabled" }
  | { status: "connecting"; address: string; error: string | null }
  | { status: "connected"; address: string }
  /** `no_host`: no broker entered. */
  | { status: "error"; message: string };

export type ApiState =
  | { status: "disabled" }
  | { status: "running"; address: string; url: string }
  | { status: "error"; port: number; message: string };

export type Bound = "low" | "high";

export interface ActiveAlert {
  metric: Metric;
  channel: number;
  bound: Bound;
  since: number;
  threshold: number;
  value: number;
}

export interface AlertEvent {
  ts: number;
  metric: Metric;
  channel: number;
  bound: Bound;
  kind: "triggered" | "recovered";
  value: number;
  threshold: number;
  notify: boolean;
}

export interface AlertLogEntry {
  ts: number;
  metric: Metric;
  channel: number;
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

/** Each metric has one series per entry of `channels`, aligned with `t` (seconds). */
export interface History {
  from: number;
  to: number;
  bucket_ms: number;
  t: number[];
  channels: number[];
  temperature: Series[];
  humidity: Series[];
  illuminance: Series[];
  pressure: Series[];
  co2: Series[];
  voc: Series[];
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
  /** true, false, or null while unknown: a latching relay cannot be read back. */
  state: boolean | null;
  /** End of a running timed pulse (Unix ms); the relay then switches back. */
  pulse_until: number | null;
}

/** What the GPS Module knows. */
export interface GpsInfo {
  channel: number;
  /** A valid 2D or 3D position. */
  fix: boolean;
  satellites: number;
  /** Degrees (WGS 84). */
  latitude: number | null;
  longitude: number | null;
  /** Metres above sea level. */
  altitude: number | null;
  /** Estimated horizontal accuracy in metres. */
  accuracy: number | null;
  /** UTC time from the satellites (Unix ms). */
  time: number | null;
}

/** Detected NFC Tag (NTAG I²C plus). */
export interface NfcInfo {
  channel: number;
  address: number;
  part: string;
  /** The URL on the tag; null when it holds none. */
  url: string | null;
  /** NDEF formatted, so a URL can be written. */
  formatted: boolean;
}

/** What the app knows about newer versions (Settings, About and the Dashboard banner). */
export interface UpdateState {
  /** A check is under way. */
  checking: boolean;
  /** The newest release, when it is newer than this version. */
  available: { version: string; url: string } | null;
  /** When the last check ended (Unix ms). */
  last_check: number | null;
  /** Why the last check failed; a release found before stays known. */
  error: string | null;
}

export interface Snapshot {
  device: DeviceState;
  relay: RelayInfo | null;
  nfc: NfcInfo | null;
  gps: GpsInfo | null;
  /** The latest reading of each I²C channel, in channel order. */
  latest: Reading[];
  settings: Settings;
  open_at_login: boolean;
  mqtt: MqttState;
  api: ApiState;
  active_alerts: ActiveAlert[];
  app: AppInfo;
  update: UpdateState;
}

export type RescanOutcome = { status: "found"; sources: SensorSource[] } | { status: "no_module" };

export interface ExportResult {
  path: string;
  rows: number;
}
