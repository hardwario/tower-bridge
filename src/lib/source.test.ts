import { describe, expect, it } from "vitest";
import { i18n } from "./i18n/index.svelte";
import {
  channelLabel,
  channelName,
  describeModules,
  lineLabels,
  metricChannels,
  metricSource,
  moduleLines,
  modulesByChannel,
  relaySource,
  sourceText,
} from "./source";
import type { DeviceState, History } from "./types";

const ready: DeviceState = {
  status: "ready",
  device: { backend: "hid", serial: "SN", path: "p", chip: null, bridges_connected: 1 },
  sources: [
    {
      metric: "temperature",
      channel: 0,
      module: "climate_module_r1",
      sensor: "TMP112",
      address: 0x48,
    },
    {
      metric: "temperature",
      channel: 1,
      module: "humidity_tag",
      sensor: "SHT20",
      address: 0x40,
    },
    {
      metric: "illuminance",
      channel: 1,
      module: "lux_meter_tag",
      sensor: "OPT3001",
      address: 0x44,
    },
  ],
};

describe("data source", () => {
  it("reads from the module over the bus and channel to the address", () => {
    i18n.set("en");
    const s = metricSource(ready, "temperature", 0);
    expect(s && sourceText(s)).toBe("Climate Module R1 · I²C (FT260) → channel 0 → 0x48 TMP112");
    i18n.set("cs");
    expect(s && sourceText(metricSource(ready, "temperature", 1)!)).toBe(
      "Humidity Tag · I²C (FT260) → kanál 1 → 0x40 SHT20",
    );
  });

  it("is unknown without a working module", () => {
    expect(metricSource({ status: "searching" }, "temperature", 0)).toBeNull();
    expect(metricSource(ready, "humidity", 0)).toBeNull();
  });

  it("lists the channels measuring a metric", () => {
    i18n.set("en");
    expect(metricChannels(ready, "temperature", null, [])).toEqual([0, 1]);
    expect(metricChannels(ready, "illuminance", null, [])).toEqual([1]);
    // Nothing known: one line on channel 0.
    expect(metricChannels({ status: "searching" }, "pressure", null, [])).toEqual([0]);
    const history: History = {
      from: 0,
      to: 1,
      bucket_ms: 1,
      t: [0],
      channels: [0, 1],
      temperature: [],
      humidity: [],
      illuminance: [],
      pressure: [
        { avg: [null], min: [null], max: [null] },
        { avg: [1000], min: [1000], max: [1000] },
      ],
      co2: [],
      voc: [],
    };
    expect(metricChannels({ status: "searching" }, "pressure", history, [])).toEqual([1]);
    expect(channelName(ready, "temperature", 1)).toBe("Humidity Tag");
    expect(channelName(ready, "pressure", 1)).toBe("Channel 1");
  });

  it("describes the modules per channel", () => {
    i18n.set("en");
    expect(describeModules(ready.status === "ready" ? ready.sources : [])).toBe(
      "Climate Module R1 (channel 0), Humidity Tag, Lux Meter Tag (channel 1)",
    );
  });

  it("merges channels from sources, readings and histories", () => {
    const empty = { avg: [null], min: [null], max: [null] };
    const h = (channels: number[], pressure: (number | null)[][]): History => ({
      from: 0,
      to: 1,
      bucket_ms: 1,
      t: [0],
      channels,
      temperature: [],
      humidity: [],
      illuminance: [],
      pressure: pressure.map((avg) => ({ ...empty, avg })),
      co2: [],
      voc: [],
    });
    const span = h([0, 1], [[1000], [1010]]);
    const zoomed = h([0], [[1000]]);
    // A zoomed window without channel 1 keeps its line.
    expect(metricChannels({ status: "searching" }, "pressure", [span, zoomed], [])).toEqual([0, 1]);
    const reading = {
      ts: 0,
      device: "SN",
      channel: 1,
      temperature: null,
      humidity: 40,
      illuminance: null,
      pressure: null,
      altitude: null,
      co2: null,
      voc: null,
    };
    expect(metricChannels({ status: "searching" }, "humidity", null, [reading])).toEqual([1]);
    expect(metricChannels({ status: "searching" }, "temperature", null, [reading])).toEqual([0]);
    expect(metricChannels({ status: "searching" }, "temperature", null, [reading], false)).toEqual([]);
  });

  it("names the channel only when two lines read the same", () => {
    i18n.set("en");
    expect(
      lineLabels([
        { channel: 0, name: "Climate Module R1" },
        { channel: 1, name: "Humidity Tag" },
      ]),
    ).toEqual(["Climate Module R1", "Humidity Tag"]);
    expect(
      lineLabels([
        { channel: 0, name: "Humidity Tag" },
        { channel: 1, name: "Humidity Tag" },
      ]),
    ).toEqual(["Humidity Tag · channel 0", "Humidity Tag · channel 1"]);
    const twins: DeviceState = {
      status: "ready",
      device: { backend: "hid", serial: "SN", path: "p", chip: null, bridges_connected: 1 },
      sources: [0, 1].map((channel) => ({
        metric: "humidity",
        channel,
        module: "humidity_tag",
        sensor: "SHT20",
        address: 0x40,
      })),
    };
    expect(channelLabel(twins, "humidity", 1)).toBe("Humidity Tag · channel 1");
    expect(channelLabel(ready, "temperature", 1)).toBe("Humidity Tag");
  });

  it("lists each module once per channel", () => {
    const sources = ready.status === "ready" ? ready.sources : [];
    // The Humidity Tag measures two values: still listed once.
    const humidity = { ...sources[1], metric: "humidity" as const };
    expect(modulesByChannel([...sources, humidity])).toEqual([
      { channel: 0, names: ["Climate Module R1"] },
      { channel: 1, names: ["Humidity Tag", "Lux Meter Tag"] },
    ]);
  });

  it("tells the same tag on both channels apart by its channel", () => {
    i18n.set("en");
    const tag = (channel: number) => ({
      metric: "humidity" as const,
      channel,
      module: "humidity_tag" as const,
      sensor: "SHT20",
      address: 0x40,
    });
    const both: DeviceState = {
      status: "ready",
      device: { backend: "hid", serial: "SN", path: "p", chip: null, bridges_connected: 1 },
      sources: [tag(0), tag(1)],
    };
    expect(channelLabel(both, "humidity", 0)).toBe("Humidity Tag · channel 0");
    expect(channelLabel(both, "humidity", 1)).toBe("Humidity Tag · channel 1");
    expect(moduleLines([tag(0), tag(1)], null).map((l) => l.channel)).toEqual([0, 1]);
  });

  it("lists every module by its product name, the Relay Module included", () => {
    const sources = ready.status === "ready" ? ready.sources : [];
    const relay = (channel: number) => ({ channel, address: 0x3b, state: null, pulse_until: null });
    const humidity = { ...sources[1], metric: "humidity" as const };
    expect(moduleLines([...sources, humidity], relay(0))).toEqual([
      { channel: 0, product: "TOWER Climate Module", revision: "R1" },
      { channel: 0, product: "TOWER Relay Module", revision: null },
      { channel: 1, product: "HARDWARIO Humidity Tag", revision: null },
      { channel: 1, product: "HARDWARIO Lux Meter Tag", revision: null },
    ]);
    // Alone on its channel, or with no sensors at all.
    expect(moduleLines(sources.slice(1), relay(0))[0]).toEqual({
      channel: 0,
      product: "TOWER Relay Module",
      revision: null,
    });
    expect(moduleLines([], relay(1))).toEqual([{ channel: 1, product: "TOWER Relay Module", revision: null }]);
    expect(moduleLines(sources, null)).toHaveLength(3);
  });

  it("names the relay's expander from its address", () => {
    const relay = (address: number) => ({ channel: 0, address, state: null, pulse_until: null });
    expect(relaySource(ready, relay(0x3b))?.part).toBe("TCA9534A");
    expect(relaySource(ready, relay(0x23))?.part).toBe("TCA9534");
    expect(relaySource({ status: "searching" }, relay(0x3b))).toBeNull();
  });
});
