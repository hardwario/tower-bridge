import type { ChartColor, ChartStyle, Metric } from "./types";

/**
 * Chart line colours, in picker order. Each has a shade per theme with at
 * least 3.7:1 contrast against that theme's card surface, and neighbours stay
 * apart under normal vision (OKLab ΔE ≥ 17) and protan/deutan simulation
 * (ΔE ≥ 9). Red is left out so a line never looks like an alert limit.
 */
export const CHART_COLORS: ChartColor[] = ["blue", "green", "pink", "gold", "violet", "teal", "orange", "graphite"];

const SHADES: Record<ChartColor, readonly [light: string, dark: string]> = {
  blue: ["#2171cc", "#4491f0"],
  green: ["#288420", "#4fa847"],
  pink: ["#b74b81", "#d7699d"],
  gold: ["#a68003", "#b58e18"],
  violet: ["#7756c0", "#9c7cea"],
  teal: ["#018b7c", "#09ab99"],
  orange: ["#c25600", "#e0702b"],
  graphite: ["#5c646f", "#8c95a0"],
};

/** A different colour per metric from the start (as in the app's settings defaults). */
const DEFAULT_COLORS: Record<Metric, ChartColor> = {
  temperature: "orange",
  humidity: "blue",
  illuminance: "gold",
  pressure: "violet",
};

const DEFAULT_LINES = { show_min: false, show_avg: true, show_max: false };

export function lineColor(color: ChartColor | undefined, dark: boolean): string {
  const shades = SHADES[color ?? "blue"] ?? SHADES.blue;
  return dark ? shades[1] : shades[0];
}

/** Translucent fill for the min/max band under a line. */
export function bandColor(hex: string, dark: boolean): string {
  const [r, g, b] = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16));
  return `rgba(${r}, ${g}, ${b}, ${dark ? 0.16 : 0.12})`;
}

export function chartStyle(charts: Partial<Record<Metric, ChartStyle>> | undefined, metric: Metric): ChartStyle {
  return { color: DEFAULT_COLORS[metric], ...DEFAULT_LINES, ...charts?.[metric] };
}
