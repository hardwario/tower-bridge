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

/**
 * A different colour per metric from the start, and a contrasting one for
 * the second I²C channel (as in the app's settings defaults; each pair passes
 * the colour-blindness checks in both themes).
 */
const DEFAULT_COLORS: Record<Metric, ChartColor[]> = {
  temperature: ["orange", "teal"],
  humidity: ["blue", "pink"],
  illuminance: ["gold", "violet"],
  pressure: ["violet", "green"],
};

const DEFAULT_LINES = { show_min: false, show_avg: true, show_max: false, hidden_channels: [] };

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
  const saved = charts?.[metric];
  const colors: ChartColor[] = [];
  DEFAULT_COLORS[metric].forEach((preferred, i) => {
    // A default never repeats a colour another channel already shows.
    const free = [preferred, ...CHART_COLORS].find((c) => !colors.includes(c)) ?? preferred;
    colors.push(saved?.colors?.[i] ?? free);
  });
  return { ...DEFAULT_LINES, ...saved, colors };
}

/** The colour of `channel`'s line. */
export function channelColor(style: ChartStyle, metric: Metric, channel: number): ChartColor {
  return style.colors[channel] ?? DEFAULT_COLORS[metric][channel % DEFAULT_COLORS[metric].length];
}

/** Of `channels` (the lines a chart has), those hidden: never all, so a single line always shows. */
export function hiddenChannels(style: ChartStyle, channels: number[]): Set<number> {
  const hidden = channels.filter((c) => style.hidden_channels?.includes(c));
  return new Set(hidden.length < channels.length ? hidden : []);
}

/** `style` with `channel`'s line hidden or shown. */
export function withChannelHidden(style: ChartStyle, channel: number, hidden: boolean): ChartStyle {
  const rest = (style.hidden_channels ?? []).filter((c) => c !== channel);
  return { ...style, hidden_channels: hidden ? [...rest, channel].sort((a, b) => a - b) : rest };
}

/** `style` with `channel`'s line in `color`. */
export function withChannelColor(style: ChartStyle, channel: number, color: ChartColor): ChartStyle {
  const colors = [...style.colors];
  colors[channel] = color;
  return { ...style, colors };
}
