import { describe, expect, it } from "vitest";
import { altitude, deltaFromDisplay, deltaToDisplay, fromDisplay, toDisplay } from "./units";

describe("units", () => {
  it("converts temperature", () => {
    expect(toDisplay("temperature", 20, "fahrenheit")).toBeCloseTo(68);
    expect(fromDisplay("temperature", 68, "fahrenheit")).toBeCloseTo(20);
    expect(toDisplay("temperature", 20, "celsius")).toBe(20);
  });

  it("converts hysteresis without offset", () => {
    expect(deltaToDisplay("temperature", 1, "fahrenheit")).toBeCloseTo(1.8);
    expect(deltaFromDisplay("temperature", 1.8, "fahrenheit")).toBeCloseTo(1);
  });

  it("converts pressure to hPa", () => {
    expect(toDisplay("pressure", 101325, "celsius")).toBeCloseTo(1013.25);
    expect(fromDisplay("pressure", 1000, "celsius")).toBe(100000);
    expect(deltaToDisplay("pressure", 100, "celsius")).toBe(1);
  });

  it("computes altitude", () => {
    expect(altitude(101325, 101325)).toBeCloseTo(0);
    expect(altitude(89874.6, 101325)).toBeCloseTo(1000, -1);
  });
});
