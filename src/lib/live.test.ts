import { describe, expect, it } from "vitest";
import { pruneLatest } from "./live.svelte";
import type { Reading, SensorSource } from "./types";

const reading = (channel: number): Reading => ({
  ts: 1,
  device: "SN",
  channel,
  temperature: 20,
  humidity: null,
  illuminance: null,
  pressure: null,
  altitude: null,
  co2: null,
  voc: null,
});

describe("live state", () => {
  it("drops the readings of a channel whose sensors are gone", () => {
    const sources: SensorSource[] = [
      {
        metric: "temperature",
        channel: 0,
        module: "climate_module_r1",
        sensor: "TMP112",
        address: 0x48,
        id: "tmp112-48",
        primary: true,
      },
    ];
    expect(pruneLatest([reading(0), reading(1)], sources).map((r) => r.channel)).toEqual([0]);
    expect(pruneLatest([reading(1)], [])).toEqual([]);
    // A further sensor's reading goes with the sensor, not with its channel.
    const tag = { ...reading(0), sensor: "sht20-40" };
    expect(pruneLatest([reading(0), tag], sources)).toEqual([reading(0)]);
    const further = { ...sources[0], id: "sht20-40", primary: false };
    expect(pruneLatest([reading(0), tag], [...sources, further])).toHaveLength(2);
  });
});
