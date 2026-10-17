export type Span = "1h" | "6h" | "24h" | "7d" | "30d" | "90d" | "365d";

const DAY = 86_400_000;

/** The time spans of the Dashboard and the Data page; longer ones refresh less often (a new reading barely moves them). */
export const SPANS: { key: Span; ms: number; refreshMs: number }[] = [
  { key: "1h", ms: 3_600_000, refreshMs: 5_000 },
  { key: "6h", ms: 6 * 3_600_000, refreshMs: 30_000 },
  { key: "24h", ms: DAY, refreshMs: 60_000 },
  { key: "7d", ms: 7 * DAY, refreshMs: 300_000 },
  { key: "30d", ms: 30 * DAY, refreshMs: 900_000 },
  { key: "90d", ms: 90 * DAY, refreshMs: 1_800_000 },
  { key: "365d", ms: 365 * DAY, refreshMs: 3_600_000 },
];

export function startOfDay(ts = Date.now()): number {
  const d = new Date(ts);
  d.setHours(0, 0, 0, 0);
  return d.getTime();
}

/** `datetime-local` input value for a timestamp. */
export function toLocalInput(ts: number): string {
  const d = new Date(ts);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

export function fromLocalInput(s: string): number {
  return new Date(s).getTime();
}
