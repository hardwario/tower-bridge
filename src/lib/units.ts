import type { Metric, TemperatureUnit } from "./types";

/** SI value (°C, %RH, lx, Pa) to display unit. */
export function toDisplay(metric: Metric, v: number, unit: TemperatureUnit): number {
  switch (metric) {
    case "temperature":
      return unit === "fahrenheit" ? (v * 9) / 5 + 32 : v;
    case "pressure":
      return v / 100;
    default:
      return v;
  }
}

/** Display unit back to SI. */
export function fromDisplay(metric: Metric, v: number, unit: TemperatureUnit): number {
  switch (metric) {
    case "temperature":
      return unit === "fahrenheit" ? ((v - 32) * 5) / 9 : v;
    case "pressure":
      return v * 100;
    default:
      return v;
  }
}

/** A difference (hysteresis) to display unit: no offset for °F. */
export function deltaToDisplay(metric: Metric, d: number, unit: TemperatureUnit): number {
  if (metric === "temperature" && unit === "fahrenheit") return (d * 9) / 5;
  if (metric === "pressure") return d / 100;
  return d;
}

export function deltaFromDisplay(metric: Metric, d: number, unit: TemperatureUnit): number {
  if (metric === "temperature" && unit === "fahrenheit") return (d * 5) / 9;
  if (metric === "pressure") return d * 100;
  return d;
}

export function unitLabel(metric: Metric, unit: TemperatureUnit): string {
  switch (metric) {
    case "temperature":
      return unit === "fahrenheit" ? "°F" : "°C";
    case "humidity":
      return "%";
    case "illuminance":
      return "lx";
    case "pressure":
      return "hPa";
    case "co2":
      return "ppm";
    case "voc":
      return "ppb";
  }
}

export function decimals(metric: Metric): number {
  return metric === "illuminance" || metric === "co2" || metric === "voc" ? 0 : 1;
}

/** Barometric altitude in metres. */
export function altitude(pressurePa: number, seaLevelPa: number): number {
  return 44330.77 * (1 - Math.pow(pressurePa / seaLevelPa, 0.190263));
}

/** Round to a sensible number of decimals for inputs. */
export function roundFor(metric: Metric, v: number): number {
  const f = metric === "illuminance" ? 1 : 10;
  return Math.round(v * f) / f;
}
