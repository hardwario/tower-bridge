import { describe, expect, it } from "vitest";
import { mqttTopics, sanitizePrefix } from "./mqtt";
import type { SensorSource } from "./types";

const src = (metric: SensorSource["metric"], channel: number): SensorSource => ({
  metric,
  channel,
  module: "lux_meter_tag",
  sensor: "X",
  address: 0,
  id: "",
  primary: true,
});

describe("MQTT topics", () => {
  it("cleans the prefix like the broker", () => {
    expect(sanitizePrefix(" /home//office/ tower #1/ ")).toBe("home/office/tower--1");
    expect(sanitizePrefix("  ")).toBe("tower");
    expect(sanitizePrefix("a/+/b")).toBe("a/-/b");
  });

  it("lists only what is measured, altitude only with pressure", () => {
    const topics = mqttTopics("tower ", [src("temperature", 0), src("pressure", 0), src("illuminance", 1)], {
      channel: 0,
      address: 0x3b,
      state: null,
      pulse_until: null,
    });
    expect(topics.map(([t]) => t)).toEqual([
      "tower/temperature/0",
      "tower/pressure/0",
      "tower/altitude/0",
      "tower/illuminance/1",
      "tower/relay/0",
      "tower/status",
    ]);
    expect(topics[0][1]).toBe("°C");
  });

  it("lists every sensor's own topics too (GitHub issue #2)", () => {
    const sensor = (metric: SensorSource["metric"], id: string, primary: boolean): SensorSource => ({
      ...src(metric, 0),
      id,
      primary,
    });
    const topics = mqttTopics(
      "tower",
      [sensor("humidity", "sht30-45", true), sensor("humidity", "sht20-40", false)],
      null,
    );
    expect(topics.map(([t]) => t)).toEqual([
      "tower/humidity/0",
      "tower/humidity/0/sht20-40",
      "tower/humidity/0/sht30-45",
      "tower/status",
    ]);
  });

  it("is empty without sensors or relay", () => {
    expect(mqttTopics("tower", [], null)).toEqual([]);
  });
});
