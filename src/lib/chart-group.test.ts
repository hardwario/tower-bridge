import { describe, expect, it } from "vitest";
import { ChartGroup, dataExtent, hasValueAt, panBounds, seriesOf, windowStats } from "./chart-group.svelte";
import type { History, Series } from "./types";

const s = (avg: (number | null)[], spread = 1): Series => ({
  avg,
  min: avg.map((v) => (v === null ? null : v - spread)),
  max: avg.map((v) => (v === null ? null : v + spread)),
});

const empty = (n: number): Series => s(Array(n).fill(null));

/** Channel 0 at t = 10..40 s; channel 1 only at 30 and 40 s. */
const history: History = {
  from: 0,
  to: 50_000,
  bucket_ms: 10_000,
  t: [10, 20, 30, 40],
  channels: [0, 1],
  temperature: [s([20, 21, 22, 23]), s([null, null, 5, 7])],
  humidity: [empty(4), empty(4)],
  illuminance: [empty(4), empty(4)],
  pressure: [empty(4), empty(4)],
  co2: [empty(4), empty(4)],
  voc: [empty(4), empty(4)],
};

describe("chart data", () => {
  it("finds a channel's series", () => {
    expect(seriesOf(history, "temperature", 1)?.avg).toEqual([null, null, 5, 7]);
    expect(seriesOf(history, "temperature", 2)).toBeNull();
    expect(seriesOf(null, "temperature", 0)).toBeNull();
  });

  it("computes statistics per channel and window", () => {
    expect(windowStats(history, "temperature", 0, null)).toEqual({
      min: 19,
      max: 24,
      avg: 21.5,
    });
    expect(windowStats(history, "temperature", 1, null)).toEqual({
      min: 4,
      max: 8,
      avg: 6,
    });
    // Only the buckets inside the zoom window count.
    expect(windowStats(history, "temperature", 0, [25, 45])).toEqual({
      min: 21,
      max: 24,
      avg: 22.5,
    });
    expect(windowStats(history, "humidity", 0, null)).toBeNull();
    expect(windowStats(history, "temperature", 3, null)).toBeNull();
  });

  it("spans the data of every channel", () => {
    expect(hasValueAt(history, 0)).toBe(true);
    const late: History = {
      ...history,
      temperature: [empty(4), s([null, 1, 2, null])],
    };
    expect(dataExtent(late)).toEqual([20, 30]);
    expect(dataExtent({ ...history, temperature: [empty(4), empty(4)] })).toBeNull();
    expect(dataExtent(null)).toBeNull();
  });

  it("lets panning reach from the oldest reading to the newest", () => {
    expect(panBounds({ first: 5_000, last: 40_000 }, history)).toEqual([5, 50]);
    expect(panBounds(null, history)).toEqual([0, 50]);
  });
});

describe("chart group", () => {
  it("keeps the window inside the data", () => {
    const g = new ChartGroup("test");
    g.bounds = [0, 1000];
    g.setWindow(-500, 200);
    expect(g.zoom).toEqual([0, 200]);
    g.setWindow(950, 200);
    expect(g.zoom).toEqual([800, 1000]);
    // Never narrower than a minute, never wider than the data.
    g.setZoom(100, 110);
    expect(g.zoom![1] - g.zoom![0]).toBe(60);
    g.setWindow(0, 5000);
    expect(g.zoom).toEqual([0, 1000]);
    g.reset();
    expect(g.zoom).toBeNull();
  });

  it("does not pan into a stretch without loaded data", () => {
    const g = new ChartGroup("test");
    g.bounds = [0, 10_000];
    g.loaded = [5000, 6000];
    g.setWindow(0, 1000);
    // At least a tenth of the window stays on the loaded data.
    expect(g.zoom![1]).toBeGreaterThanOrEqual(5000 + 100);
  });

  it("zooms around the pointer", () => {
    const g = new ChartGroup("test");
    g.bounds = [0, 10_000];
    g.home = [0, 1000];
    g.zoomAt(500, 0.5);
    expect(g.zoom).toEqual([250, 750]);
  });
});
