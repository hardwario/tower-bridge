import { describe, expect, it } from "vitest";
import {
  CHART_COLORS,
  bandColor,
  channelColor,
  chartStyle,
  hiddenChannels,
  lineColor,
  withChannelColor,
  withChannelHidden,
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
    const next = withChannelColor(style, 1, "graphite");
    expect(next.colors).toEqual(["violet", "graphite"]);
    expect(style.colors).toEqual(["violet", "green"]);
    expect(channelColor(next, "pressure", 1)).toBe("graphite");
    expect(channelColor(style, "pressure", 5)).toBe("green");
  });

  it("hides and shows a channel's line, never all of them", () => {
    const style = chartStyle(undefined, "humidity");
    expect(style.hidden_channels).toEqual([]);
    const off = withChannelHidden(style, 1, true);
    expect(off.hidden_channels).toEqual([1]);
    expect(style.hidden_channels).toEqual([]);
    expect([...hiddenChannels(off, [0, 1])]).toEqual([1]);
    // A single line, or every line hidden: all show.
    expect(hiddenChannels(off, [1]).size).toBe(0);
    expect(hiddenChannels(withChannelHidden(off, 0, true), [0, 1]).size).toBe(0);
    expect(withChannelHidden(withChannelHidden(off, 0, true), 1, false).hidden_channels).toEqual([0]);
    expect(withChannelHidden(off, 1, true).hidden_channels).toEqual([1]);
  });

  it("makes a translucent band from a line colour", () => {
    expect(bandColor("#2171cc", false)).toBe("rgba(33, 113, 204, 0.12)");
    expect(bandColor("#2171cc", true)).toBe("rgba(33, 113, 204, 0.16)");
  });
});
