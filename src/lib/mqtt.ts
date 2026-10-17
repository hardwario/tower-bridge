import type { GpsInfo, Metric, RelayInfo, SensorSource } from "./types";

/** Like the app's broker does: no wildcards or spaces, no empty levels; `tower` when nothing is left. */
export function sanitizePrefix(prefix: string): string {
  const levels = prefix
    .split("/")
    .map((l) => l.trim().replace(/[+# ]/g, "-"))
    .filter((l) => l.length > 0);
  return levels.length ? levels.join("/") : "tower";
}

const UNITS: Record<Metric | "altitude", string> = {
  temperature: "°C",
  humidity: "%",
  illuminance: "lx",
  pressure: "Pa",
  altitude: "m",
  co2: "ppm",
  voc: "ppb",
};
const ORDER: (Metric | "altitude")[] = ["temperature", "humidity", "illuminance", "pressure", "altitude", "co2", "voc"];

/**
 * The topics published now, with their units: each measured value on its
 * channel (altitude where there is pressure), the relay and the app's status.
 */
export function mqttTopics(
  prefix: string,
  sources: SensorSource[],
  relay: RelayInfo | null,
  gps: GpsInfo | null = null,
): [string, string][] {
  const p = sanitizePrefix(prefix);
  const rows: [string, string][] = [];
  const channels = [...new Set(sources.map((s) => s.channel))].sort((a, b) => a - b);
  for (const c of channels) {
    const measured = new Set<string>(sources.filter((s) => s.channel === c).map((s) => s.metric));
    if (measured.has("pressure")) measured.add("altitude");
    for (const m of ORDER) if (measured.has(m)) rows.push([`${p}/${m}/${c}`, UNITS[m]]);
  }
  if (relay) rows.push([`${p}/relay/${relay.channel}`, "true | false | null"]);
  if (gps) {
    const g = `${p}/gps/${gps.channel}`;
    rows.push([`${g}/latitude`, "°"], [`${g}/longitude`, "°"], [`${g}/altitude`, "m"], [`${g}/satellites`, ""]);
  }
  if (rows.length) rows.push([`${p}/status`, "online | offline"]);
  return rows;
}
