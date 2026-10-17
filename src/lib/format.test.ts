import { describe, expect, it } from "vitest";
import { excelDateTimeFormat, fmtClock, fmtDate, fmtDateTime, fmtDayMonth, fmtDuration, fmtInput, fmtNumber, parseInput } from "./format";
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
