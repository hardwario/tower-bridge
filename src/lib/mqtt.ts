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
 * channel (altitude where there is pressure), then once more one level down
 * per sensor (`tower/humidity/0/sht20-40`, so a Humidity Tag next to a
 * Climate Module has its own), the relay and the app's status.
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
  const withAltitude = (list: SensorSource[]) => {
    const measured = new Set<string>(list.map((s) => s.metric));
    if (measured.has("pressure")) measured.add("altitude");
    return ORDER.filter((m) => measured.has(m));
  };
  for (const c of channels) {
    const here = sources.filter((s) => s.channel === c);
    for (const m of withAltitude(here.filter((s) => s.primary !== false))) rows.push([`${p}/${m}/${c}`, UNITS[m]]);
    const ids = [...new Set(here.map((s) => s.id).filter((id) => id))].sort();
    for (const id of ids) {
      for (const m of withAltitude(here.filter((s) => s.id === id))) rows.push([`${p}/${m}/${c}/${id}`, UNITS[m]]);
    }
  }
  if (relay) rows.push([`${p}/relay/${relay.channel}`, "true | false | null"]);
  if (gps) {
    const g = `${p}/gps/${gps.channel}`;
    rows.push([`${g}/latitude`, "°"], [`${g}/longitude`, "°"], [`${g}/altitude`, "m"], [`${g}/satellites`, ""]);
  }
  if (rows.length) rows.push([`${p}/status`, "online | offline"]);
  return rows;
}
