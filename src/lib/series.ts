import type { History, Reading, SensorSource } from "./types";

/**
 * A line of a chart, a column of the Data page: a channel's own series ("0"),
 * or the series of a further sensor on it ("0/sht20-40", a Humidity Tag next
 * to a Climate Module).
 */
export type LineKey = string;

export const lineKey = (channel: number, sensor?: string | null): LineKey =>
  sensor ? `${channel}/${sensor}` : `${channel}`;

export function parseLine(key: LineKey): { channel: number; sensor: string | null } {
  const [channel, sensor] = key.split("/", 2);
  return { channel: Number(channel), sensor: sensor || null };
}

export const readingKey = (r: Reading): LineKey => lineKey(r.channel, r.sensor);

export const sourceKey = (s: SensorSource): LineKey => lineKey(s.channel, s.primary === false ? s.id : null);

/** The key of each series of a history, in its order. */
export const historyKeys = (h: History): LineKey[] => h.channels.map((c, i) => lineKey(c, h.sensors?.[i]));

/** By channel, a channel's own line first, then further sensors by id. */
export function compareLines(a: LineKey, b: LineKey): number {
  const x = parseLine(a);
  const y = parseLine(b);
  if (x.channel !== y.channel) return x.channel - y.channel;
  if (x.sensor === y.sensor) return 0;
  if (x.sensor === null) return -1;
  if (y.sensor === null) return 1;
  return x.sensor.localeCompare(y.sensor);
}
