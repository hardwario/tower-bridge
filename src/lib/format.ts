import { i18n } from "./i18n/index.svelte";
import type { DateFormat, Metric, TemperatureUnit, TimeFormat } from "./types";
import { decimals, toDisplay, unitLabel } from "./units";

const cache = new Map<string, Intl.NumberFormat | Intl.DateTimeFormat>();

function cached<T extends Intl.NumberFormat | Intl.DateTimeFormat>(
  key: string,
  make: () => T,
): T {
  const full = `${i18n.locale}|${key}`;
  let f = cache.get(full) as T | undefined;
  if (!f) {
    f = make();
    cache.set(full, f);
  }
  return f;
}

/**
 * With the decimal separator chosen in the settings instead of the locale's.
 * A digit group separator that would then look like it gives way to a narrow
 * space (12 400.5, never 12,400,5).
 */
function separated(f: Intl.NumberFormat, v: number): string {
  if (i18n.decimal === "auto") return f.format(v);
  const decimal = i18n.decimal === "comma" ? "," : ".";
  return f
    .formatToParts(v)
    .map((p) => (p.type === "decimal" ? decimal : p.type === "group" && p.value === decimal ? "\u202f" : p.value))
    .join("");
}

/** Locale number; digit grouping only from 10 000 up (1013,2 hPa, 12 400 lx). */
export function fmtNumber(v: number, d: number): string {
  const grouping = Math.abs(v) >= 10_000;
  const f = cached(
    `n${d}${grouping}`,
    () =>
      new Intl.NumberFormat(i18n.locale, {
        minimumFractionDigits: d,
        maximumFractionDigits: d,
        useGrouping: grouping,
      }),
  );
  return separated(f, v);
}

/** Number for an input field, with the chosen decimal separator. */
export function fmtInput(v: number): string {
  const f = cached(
    "input",
    () =>
      new Intl.NumberFormat(i18n.locale, {
        useGrouping: false,
        maximumFractionDigits: 3,
      }),
  );
  return separated(f, v);
}

/** Parse user input accepting "," or "." as decimal separator. */
export function parseInput(s: string): number {
  const text = s.trim().replace(/\s/g, "").replace(",", ".");
  // Number("") is 0: an emptied field is no value.
  return text === "" ? NaN : Number(text);
}

/** Shown where a value is missing. */
export const NO_VALUE = "-";

/** Display number without unit, or NO_VALUE. */
export function fmtValue(metric: Metric, v: number | null | undefined, unit: TemperatureUnit): string {
  if (v === null || v === undefined || Number.isNaN(v)) return NO_VALUE;
  return fmtNumber(toDisplay(metric, v, unit), decimals(metric));
}

export function fmtWithUnit(metric: Metric, v: number | null | undefined, unit: TemperatureUnit): string {
  const s = fmtValue(metric, v, unit);
  return s === NO_VALUE ? s : `${s} ${unitLabel(metric, unit)}`;
}

const pad = (n: number) => String(n).padStart(2, "0");

/** Intl options for the chosen 12/24-hour clock ("auto" follows the locale). */
function clockOptions(seconds: boolean, format: TimeFormat): Intl.DateTimeFormatOptions {
  const o: Intl.DateTimeFormatOptions = { hour: "numeric", minute: "2-digit" };
  if (seconds) o.second = "2-digit";
  if (format === "h24") o.hourCycle = "h23";
  if (format === "h12") o.hourCycle = "h12";
  return o;
}

/** Clock time: "14:05", "2:05 PM", optionally with seconds. */
export function fmtClock(ts: number, seconds = false, format: TimeFormat = i18n.timeFormat): string {
  return cached(`clock${seconds}${format}`, () => new Intl.DateTimeFormat(i18n.locale, clockOptions(seconds, format))).format(
    ts,
  );
}

/** Full date in the chosen format: 2026-10-17, 17.10.2026, 17/10/2026, 10/17/2026 or the locale's. */
export function fmtDate(ts: number, format: DateFormat = i18n.dateFormat): string {
  const d = new Date(ts);
  const [y, m, day] = [d.getFullYear(), pad(d.getMonth() + 1), pad(d.getDate())];
  switch (format) {
    case "iso":
      return `${y}-${m}-${day}`;
    case "dmy":
      return `${day}.${m}.${y}`;
    case "dmy_slash":
      return `${day}/${m}/${y}`;
    case "mdy":
      return `${m}/${day}/${y}`;
    default:
      return cached(
        "date",
        () =>
          new Intl.DateTimeFormat(i18n.locale, {
            year: "numeric",
            month: "numeric",
            day: "numeric",
          }),
      ).format(ts);
  }
}

/** Day and month without the year (chart axis): 10-17, 17.10., 17/10, 10/17 or "17 Oct". */
export function fmtDayMonth(ts: number): string {
  const d = new Date(ts);
  const [m, day] = [pad(d.getMonth() + 1), pad(d.getDate())];
  switch (i18n.dateFormat) {
    case "iso":
      return `${m}-${day}`;
    case "dmy":
      return `${day}.${m}.`;
    case "dmy_slash":
      return `${day}/${m}`;
    case "mdy":
      return `${m}/${day}`;
    default:
      return cached(
        "dayMonth",
        () =>
          new Intl.DateTimeFormat(i18n.locale, {
            month: "short",
            day: "numeric",
          }),
      ).format(ts);
  }
}

/** Date and time; without the year unless asked for. */
export function fmtDateTime(ts: number, { year = false, seconds = false } = {}): string {
  if (i18n.dateFormat === "auto") {
    // Let the locale join date and time ("Oct 17, 2:05 PM", "17. 10. 2026 14:05:09").
    const key = `dt${year}${seconds}${i18n.timeFormat}`;
    return cached(
      key,
      () =>
        new Intl.DateTimeFormat(i18n.locale, {
          ...(year ? { year: "numeric", month: "numeric" } : { month: "short" }),
          day: "numeric",
          ...clockOptions(seconds, i18n.timeFormat),
        }),
    ).format(ts);
  }
  return `${year ? fmtDate(ts) : fmtDayMonth(ts)} ${fmtClock(ts, seconds)}`;
}

/** Excel number format matching the chosen date and time formats (XLSX export). */
export function excelDateTimeFormat(): string {
  // "/" in an Excel format means the reader's date separator; escape it to keep it literal.
  const literal = (s: string) => s.replace(/\s/g, " ").replace(/\//g, "\\/");
  const fixed: Partial<Record<DateFormat, string>> = {
    iso: "yyyy-mm-dd",
    dmy: "dd.mm.yyyy",
    dmy_slash: "dd\\/mm\\/yyyy",
    mdy: "mm\\/dd\\/yyyy",
  };
  const date =
    fixed[i18n.dateFormat] ??
    new Intl.DateTimeFormat(i18n.locale, { year: "numeric", month: "numeric", day: "numeric" })
      .formatToParts(new Date(2001, 0, 1))
      .map((p) => {
        if (p.type === "day") return p.value.length > 1 ? "dd" : "d";
        if (p.type === "month") return p.value.length > 1 ? "mm" : "m";
        if (p.type === "year") return "yyyy";
        return literal(p.value);
      })
      .join("");
  const cycle = new Intl.DateTimeFormat(i18n.locale, clockOptions(true, i18n.timeFormat)).resolvedOptions().hourCycle;
  return `${date} ${cycle === "h12" || cycle === "h11" ? "h:mm:ss AM/PM" : "hh:mm:ss"}`;
}

/** Chart axis label: clock time for short ranges, date for long ones. */
export function fmtAxis(ts: number, spanMs: number): string {
  // Zoomed into minutes, ticks are seconds apart: show them.
  if (spanMs <= 600_000) return fmtClock(ts, true);
  return spanMs <= 2 * 86_400_000 ? fmtClock(ts) : fmtDayMonth(ts);
}

/** Compact duration: "10 s", "5 min", "2 h". */
export function fmtDuration(ms: number): string {
  const s = Math.round(ms / 1000);
  if (s < 60) return `${s} s`;
  if (s < 3600) return `${fmtNumber(s / 60, s % 60 ? 1 : 0)} min`;
  return `${fmtNumber(s / 3600, s % 3600 ? 1 : 0)} h`;
}
