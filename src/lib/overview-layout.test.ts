import { describe, expect, it } from "vitest";
import { mergeOrder } from "./overview-layout.svelte";

describe("dashboard layout", () => {
  it("keeps the saved order and puts a new panel at its default place", () => {
    // Saved before the NFC Tag card existed, when the Bridge Module card was a
    // panel too: the new panels go to their default places, the card is dropped.
    const saved = ["temperature", "relay", "humidity", "illuminance", "pressure", "device", "nfc"] as never[];
    expect(mergeOrder(saved)).toEqual([
      "temperature",
      "relay",
      "humidity",
      "illuminance",
      "pressure",
      "co2",
      "voc",
      "nfc",
      "gps",
    ]);
  });

  it("starts from the defaults and drops what it does not know", () => {
    expect(mergeOrder([])).toEqual([
      "relay",
      "temperature",
      "humidity",
      "illuminance",
      "pressure",
      "co2",
      "voc",
      "nfc",
      "gps",
    ]);
    expect(mergeOrder(["device" as never, "bogus" as never, "relay", "relay"])).toEqual([
      "relay",
      "temperature",
      "humidity",
      "illuminance",
      "pressure",
      "co2",
      "voc",
      "nfc",
      "gps",
    ]);
  });
});
