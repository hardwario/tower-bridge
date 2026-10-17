import { describe, expect, it } from "vitest";
import { i18n } from "./i18n/index.svelte";
import { metricSource, sourceText } from "./source";
import type { DeviceState } from "./types";

const ready: DeviceState = {
  status: "ready",
  device: { backend: "hid", serial: "SN", path: "p", chip: null },
  revision: "R1",
  channel: 0,
  sources: [{ metric: "temperature", sensor: "TMP112", address: 0x48 }],
};

describe("data source", () => {
  it("reads from the module over the bus and channel to the address", () => {
    i18n.set("en");
    const s = metricSource(ready, "temperature");
    expect(s && sourceText(s)).toBe("Climate Module R1 · I²C (FT260) → channel 0 → 0x48 TMP112");
    i18n.set("cs");
    expect(s && sourceText(metricSource(ready, "temperature")!)).toBe(
      "Climate Module R1 · I²C (FT260) → kanál 0 → 0x48 TMP112",
    );
  });

  it("is unknown without a working module", () => {
    expect(metricSource({ status: "searching" }, "temperature")).toBeNull();
    expect(metricSource(ready, "humidity")).toBeNull();
  });
});
