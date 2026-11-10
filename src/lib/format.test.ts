import { describe, expect, it } from "vitest";
import { excelDateTimeFormat, fmtAxis, fmtClock, fmtDate, fmtDateTime, fmtDayMonth, fmtDuration, fmtInput, fmtNumber, parseInput } from "./format";
import { i18n } from "./i18n/index.svelte";

describe("format", () => {
  it("uses the language's decimal separator", () => {
    i18n.set("cs");
    expect(fmtNumber(1013.25, 1)).toBe("1013,3");
    expect(fmtInput(23.5)).toBe("23,5");
    i18n.set("de");
    expect(fmtNumber(12400, 0)).toBe("12.400");
    i18n.set("en");
    expect(fmtNumber(1013.25, 1)).toBe("1013.3");
  });

  it("uses the separator chosen in the settings", () => {
    try {
      i18n.set("en");
      i18n.decimal = "comma";
      expect(fmtNumber(1013.25, 1)).toBe("1013,3");
      expect(fmtInput(23.5)).toBe("23,5");
      // An English group comma would read as a decimal one: a narrow space instead.
      expect(fmtNumber(12400.5, 1)).toBe("12\u202f400,5");
      i18n.set("de");
      i18n.decimal = "point";
      expect(fmtNumber(23.45, 1)).toBe("23.5");
      expect(fmtNumber(12400.5, 1)).toBe("12\u202f400.5");
      i18n.set("cs");
      expect(fmtNumber(12400.5, 1)).toBe("12\u00a0400.5");
    } finally {
      i18n.decimal = "auto";
      i18n.set("en");
    }
  });

  it("never shows a minus zero", () => {
    i18n.set("cs");
    expect(fmtNumber(-0.04, 1)).toBe("0,0");
    expect(fmtNumber(-0.05, 1)).toBe("-0,1");
    expect(fmtNumber(-1e-16, 0)).toBe("0");
    i18n.set("en");
    expect(fmtNumber(-0.4, 0)).toBe("0");
    expect(fmtNumber(-0.6, 0)).toBe("-1");
  });

  it("labels axis ticks hours apart over several days with the date once a day", () => {
    i18n.set("en");
    const midnight = new Date(2026, 10, 6, 0, 0).getTime();
    const noon = new Date(2026, 10, 6, 12, 0).getTime();
    const threeDays = 3 * 86_400_000;
    expect(fmtAxis(midnight, threeDays, 12 * 3_600_000)).toBe(fmtDayMonth(midnight));
    expect(fmtAxis(noon, threeDays, 12 * 3_600_000)).toBe(fmtClock(noon));
    // A tick a day or more apart: always the date.
    expect(fmtAxis(noon, 30 * 86_400_000, 86_400_000)).toBe(fmtDayMonth(noon));
    expect(fmtAxis(noon, 86_400_000)).toBe(fmtClock(noon));
  });

  it("parses either separator", () => {
    expect(parseInput("23,5")).toBe(23.5);
    expect(parseInput(" 1013.25 ")).toBe(1013.25);
  });

  it("formats dates and times in the chosen format", () => {
    const ts = new Date(2026, 9, 7, 14, 5, 9).getTime();
    i18n.set("cs");
    i18n.timeFormat = "h24";
    i18n.dateFormat = "iso";
    expect(fmtDate(ts)).toBe("2026-10-07");
    expect(fmtDayMonth(ts)).toBe("10-07");
    expect(fmtDateTime(ts, { year: true, seconds: true })).toBe("2026-10-07 14:05:09");
    i18n.dateFormat = "dmy";
    expect(fmtDate(ts)).toBe("07.10.2026");
    expect(fmtDayMonth(ts)).toBe("07.10.");
    i18n.dateFormat = "dmy_slash";
    expect(fmtDate(ts)).toBe("07/10/2026");
    i18n.dateFormat = "mdy";
    expect(fmtDate(ts)).toBe("10/07/2026");
    i18n.set("en");
    i18n.timeFormat = "h12";
    expect(fmtClock(ts)).toMatch(/^2:05\sPM$/);
    i18n.timeFormat = "h24";
    expect(fmtClock(ts, true)).toBe("14:05:09");
    i18n.dateFormat = "auto";
    i18n.timeFormat = "auto";
    i18n.set("cs");
    expect(fmtDateTime(ts, { year: true, seconds: true })).toBe("7. 10. 2026 14:05:09");
  });

  it("builds matching Excel number formats", () => {
    i18n.set("cs");
    expect(excelDateTimeFormat()).toBe("d. m. yyyy hh:mm:ss");
    i18n.set("de");
    expect(excelDateTimeFormat()).toBe("d.m.yyyy hh:mm:ss");
    i18n.dateFormat = "mdy";
    i18n.timeFormat = "h12";
    expect(excelDateTimeFormat()).toBe("mm\\/dd\\/yyyy h:mm:ss AM/PM");
    i18n.dateFormat = "auto";
    i18n.timeFormat = "auto";
  });

  it("formats durations", () => {
    i18n.set("en");
    expect(fmtDuration(10_000)).toBe("10 s");
    expect(fmtDuration(300_000)).toBe("5 min");
    expect(fmtDuration(7_200_000)).toBe("2 h");
  });
});
