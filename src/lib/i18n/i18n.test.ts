import { describe, expect, it } from "vitest";
import { cs } from "./cs";
import { de } from "./de";
import { en } from "./en";
import { fill, i18n, plural } from "./index.svelte";
import { pl } from "./pl";
import { sk } from "./sk";

function keys(o: object, prefix = ""): string[] {
  return Object.entries(o).flatMap(([k, v]) =>
    v && typeof v === "object" && !("one" in v) ? keys(v, `${prefix}${k}.`) : [`${prefix}${k}`],
  );
}

describe("i18n", () => {
  it("every language has the same keys as English", () => {
    const reference = keys(en).sort();
    for (const dict of [cs, sk, de, pl]) expect(keys(dict).sort()).toEqual(reference);
  });

  it("keeps placeholders in translations", () => {
    const placeholders = (s: string) => (s.match(/\{\w+\}/g) ?? []).sort().join();
    const flat = (o: object): Record<string, string> =>
      Object.fromEntries(
        Object.entries(o).flatMap(([k, v]) =>
          typeof v === "string" ? [[k, v]] : Object.entries(flat(v)).map(([kk, vv]) => [`${k}.${kk}`, vv]),
        ),
      );
    const ref = flat(en);
    for (const dict of [cs, sk, de, pl]) {
      const f = flat(dict);
      for (const [k, v] of Object.entries(ref)) {
        if (k.includes(".one") || k.includes(".few") || k.includes(".many") || k.includes(".other")) continue;
        expect(placeholders(f[k] ?? ""), k).toBe(placeholders(v));
      }
    }
  });

  it("fills placeholders", () => {
    expect(fill("Updated {ago}", { ago: "now" })).toBe("Updated now");
    expect(fill("keep {unknown}", {})).toBe("keep {unknown}");
  });

  it("uses Czech plural forms", () => {
    i18n.set("cs");
    expect(plural(cs.units.minutes, 1)).toBe("1 minuta");
    expect(plural(cs.units.minutes, 3)).toBe("3 minuty");
    expect(plural(cs.units.minutes, 5)).toBe("5 minut");
    expect(plural(cs.units.years, 2)).toBe("2 roky");
  });

  it("uses Polish plural forms", () => {
    i18n.set("pl");
    expect(plural(pl.units.days, 1)).toBe("1 dzień");
    expect(plural(pl.units.years, 22)).toBe("22 lata");
    expect(plural(pl.units.years, 25)).toBe("25 lat");
  });

  it("uses English and German plural forms", () => {
    i18n.set("de");
    expect(plural(de.units.days, 1)).toBe("1 Tag");
    expect(plural(de.units.days, 7)).toBe("7 Tage");
    i18n.set("en");
    expect(plural(en.units.seconds, 30)).toBe("30 seconds");
  });
});
