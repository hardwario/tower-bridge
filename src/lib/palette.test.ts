import { describe, expect, it } from "vitest";
import {
  CHART_COLORS,
  bandColor,
  chartStyle,
  hiddenLines,
  lineColor,
  lineColorOf,
  withLineColor,
  withLineHidden,
} from "./palette";

describe("chart palette", () => {
  it("has a light and a dark shade for every colour", () => {
    for (const c of CHART_COLORS) {
      expect(lineColor(c, false)).toMatch(/^#[0-9a-f]{6}$/);
      expect(lineColor(c, true)).toMatch(/^#[0-9a-f]{6}$/);
      expect(lineColor(c, true)).not.toBe(lineColor(c, false));
    }
  });

  it("starts with a colour per metric and a contrasting one for channel 1", () => {
    const t = chartStyle(undefined, "temperature");
    expect(t.colors).toEqual(["orange", "teal"]);
    expect(t.show_avg).toBe(true);
    for (const m of ["temperature", "humidity", "illuminance", "pressure"] as const) {
      const [a, b] = chartStyle(undefined, m).colors;
      expect(a).not.toBe(b);
    }
  });

  it("keeps saved colours and fills in missing ones", () => {
    const saved = {
      colors: ["pink" as const],
      show_min: true,
      show_avg: false,
      show_max: false,
      hidden_channels: [],
    };
    const style = chartStyle({ humidity: saved }, "humidity");
    // Channel 1's default (pink) is taken: the first free colour instead.
    expect(style.colors).toEqual(["pink", "blue"]);
    expect(style.show_min).toBe(true);
    const fresh = chartStyle({ humidity: { ...saved, colors: [] } }, "humidity");
    expect(fresh.colors).toEqual(["blue", "pink"]);
  });

  it("changes one channel's colour", () => {
    const style = chartStyle(undefined, "pressure");
    const next = withLineColor(style, "1", "graphite");
    expect(next.colors).toEqual(["violet", "graphite"]);
    expect(style.colors).toEqual(["violet", "green"]);
    expect(lineColorOf(next, "pressure", "1")).toBe("graphite");
    expect(lineColorOf(style, "pressure", "5")).toBe("green");
  });

  it("gives a further sensor a colour of its own (GitHub issue #2)", () => {
    const style = chartStyle(undefined, "humidity");
    const lines = ["0", "0/sht20-40", "0/sht30-44", "1"];
    // Free colours after the channels' blue and pink, each sensor its own.
    expect(lineColorOf(style, "humidity", "0/sht20-40", lines)).toBe("green");
    expect(lineColorOf(style, "humidity", "0/sht30-44", lines)).toBe("gold");
    const picked = withLineColor(style, "0/sht20-40", "teal");
    expect(picked.colors).toEqual(style.colors);
    expect(picked.sensor_colors).toEqual({ "0/sht20-40": "teal" });
    expect(lineColorOf(picked, "humidity", "0/sht20-40", lines)).toBe("teal");
    const off = withLineHidden(style, "0/sht20-40", true);
    expect(off.hidden_sensors).toEqual(["0/sht20-40"]);
    expect(off.hidden_channels).toEqual([]);
    expect([...hiddenLines(off, lines)]).toEqual(["0/sht20-40"]);
  });

  it("hides and shows a channel's line, never all of them", () => {
    const style = chartStyle(undefined, "humidity");
    expect(style.hidden_channels).toEqual([]);
    const off = withLineHidden(style, "1", true);
    expect(off.hidden_channels).toEqual([1]);
    expect(style.hidden_channels).toEqual([]);
    expect([...hiddenLines(off, ["0", "1"])]).toEqual(["1"]);
    // A single line, or every line hidden: all show.
    expect(hiddenLines(off, ["1"]).size).toBe(0);
    expect(hiddenLines(withLineHidden(off, "0", true), ["0", "1"]).size).toBe(0);
    expect(withLineHidden(withLineHidden(off, "0", true), "1", false).hidden_channels).toEqual([0]);
    expect(withLineHidden(off, "1", true).hidden_channels).toEqual([1]);
  });

  it("makes a translucent band from a line colour", () => {
    expect(bandColor("#2171cc", false)).toBe("rgba(33, 113, 204, 0.12)");
    expect(bandColor("#2171cc", true)).toBe("rgba(33, 113, 204, 0.16)");
  });
});
