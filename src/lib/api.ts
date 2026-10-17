import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  ActiveAlert,
  AlertEvent,
  AlertLogEntry,
  ApiState,
  DeviceState,
  ExportResult,
  GpsInfo,
  History,
  MqttState,
  NfcInfo,
  Reading,
  RelayInfo,
  RescanOutcome,
  Settings,
  Snapshot,
} from "./types";

export const api = {
  snapshot: () => invoke<Snapshot>("get_snapshot"),
  /** Oldest and newest stored reading (Unix ms), or null without data. */
  dataRange: () => invoke<{ first: number; last: number } | null>("get_data_range"),
  history: (from: number, to: number, maxPoints?: number) =>
    invoke<History>("get_history", { from, to, max_points: maxPoints }),
  alertLog: (limit = 50) => invoke<AlertLogEntry[]>("get_alert_log", { limit }),
  saveSettings: (settings: Settings) => invoke<Settings>("save_settings", { settings }),
  setOpenAtLogin: (enabled: boolean) => invoke<boolean>("set_open_at_login", { enabled }),
  exportCsv: (from: number, to: number) => invoke<ExportResult | null>("export_csv", { from, to }),
  exportXlsx: (from: number, to: number, datetimeFormat: string) =>
    invoke<ExportResult | null>("export_xlsx", { from, to, datetime_format: datetimeFormat }),
  testNotification: () => invoke<void>("send_test_notification"),
  identify: () => invoke<void>("identify_device"),
  rescan: () => invoke<RescanOutcome>("rescan_device"),
  /** Switch the relay to true or false. */
  setRelay: (state: boolean) => invoke<RelayInfo>("set_relay", { state }),
  /** Write a URL to the NFC Tag; resolves with what the tag holds afterwards. */
  writeNfcUrl: (url: string) => invoke<NfcInfo>("write_nfc_url", { url }),
  /** Open a position on OpenStreetMap in the system browser. */
  openMap: (latitude: number, longitude: number) => invoke<void>("open_map", { latitude, longitude }),
  /** Timed pulse to `state` (default: the opposite one) for `durationMs` (default: the setting). */
  pulseRelay: (state?: boolean, durationMs?: number) =>
    invoke<RelayInfo>("pulse_relay", { state: state ?? null, duration_ms: durationMs ?? null }),
  openDataFolder: () => invoke<void>("open_data_folder"),
  revealFile: (path: string) => invoke<void>("reveal_file", { path }),
  /** Open docs / store / academy in the system browser. */
  openLink: (target: "docs" | "store" | "academy") => invoke<void>("open_link", { target }),
};

export interface Events {
  reading: Reading;
  "device-state": DeviceState;
  "relay-state": RelayInfo | null;
  "nfc-state": NfcInfo | null;
  "gps-state": GpsInfo | null;
  "settings-changed": Settings;
  "open-at-login-changed": boolean;
  alert: AlertEvent;
  "alerts-active": ActiveAlert[];
  "mqtt-state": MqttState;
  "api-state": ApiState;
  "export-progress": { done: number; total: number };
}

export function on<K extends keyof Events>(
  event: K,
  handler: (payload: Events[K]) => void,
): Promise<UnlistenFn> {
  return listen<Events[K]>(event, (e) => handler(e.payload));
}

/** Machine-readable code of a command error (e.g. "not_connected"). */
export function errorCode(e: unknown): string | undefined {
  if (e && typeof e === "object" && "code" in e) return String((e as { code: unknown }).code);
  return undefined;
}

/** Message of an error rejected by a Tauri command. */
export function errorMessage(e: unknown): string {
  if (e && typeof e === "object" && "message" in e) return String((e as { message: unknown }).message);
  return String(e);
}
