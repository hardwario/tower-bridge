import { describe, expect, it } from "vitest";
import { cs } from "./i18n/cs";
import { de } from "./i18n/de";
import { en } from "./i18n/en";
import type { Lang, Messages } from "./i18n/index.svelte";
import { pl } from "./i18n/pl";
import { sk } from "./i18n/sk";
import {
  diagramLabels,
  hasImage,
  imagePaths,
  label,
  manualSource,
  NOTICES,
  renderManual,
  renderNotices,
  sectionIds,
} from "./manual";

const DICTS: Record<Lang, Messages> = { en, cs, sk, de, pl };
const LANGS = Object.keys(DICTS) as Lang[];

/** UI labels the manual names; renaming one in the app must update every manual. */
const LABELS = [
  "nav.overview",
  "overview.lastUpdate",
  "overview.identify",
  "overview.rescan",
  "relay.activateTrue",
  "relay.activateFalse",
  "relay.pulseTrue",
  "relay.pulseFalse",
  "relay.pulseDuration",
  "relay.unknown",
  "data.exportCsv",
  "data.exportXlsx",
  "alerts.below",
  "alerts.above",
  "alerts.hysteresis",
  "alerts.repeatAfter",
  "alerts.notifyRecovery",
  "alerts.disconnected",
  "alerts.sendTest",
  "settings.general",
  "settings.measurement",
  "settings.data",
  "settings.mqtt.allowControl",
  "settings.api.allowControl",
  "banners.noModuleTitle",
  "settings.mqtt.host",
  "overview.interval",
  "nfc.write",
  "manual.notices",
];

describe("user manual", () => {
  it("exists in every language with the same sections", () => {
    const reference = sectionIds(manualSource("en"));
    expect(reference.length).toBeGreaterThan(10);
    for (const lang of LANGS) expect(sectionIds(manualSource(lang)), lang).toEqual(reference);
  });

  it("uses the same images in every language", () => {
    const reference = imagePaths(manualSource("en")).map((p) => p.replace("/en/", "/{lang}/"));
    for (const lang of LANGS) {
      const paths = imagePaths(manualSource(lang));
      expect(
        paths.map((p) => p.replace(`/${lang}/`, "/{lang}/")),
        lang,
      ).toEqual(reference);
      for (const path of paths) {
        expect(hasImage(path), path).toBe(true);
        if (path.startsWith("images/")) expect(hasImage(path.replace(/\.webp$/, ".dark.webp")), path).toBe(true);
      }
    }
  });

  it("names the UI labels as the app shows them", () => {
    for (const lang of LANGS) {
      const source = manualSource(lang);
      for (const key of LABELS) {
        const text = label(DICTS[lang], key);
        expect(text, `${lang} ${key}`).toBeDefined();
        expect(source.includes(text ?? ""), `${lang}.md lacks "${text}" (${key})`).toBe(true);
      }
    }
  });

  it("links only to its own sections and the notices", () => {
    for (const lang of LANGS) {
      const source = manualSource(lang);
      const ids = new Set([...sectionIds(source), NOTICES]);
      for (const m of source.matchAll(/\]\(#([^)]+)\)/g))
        expect(ids.has(m[1]), `${lang}.md links to #${m[1]}`).toBe(true);
    }
  });

  it("shows the third-party notices as text, the license texts folded", () => {
    const html = renderNotices(
      "# Third-party notices\n\n- serde 1.0.0: MIT, https://github.com/serde-rs/serde\n\n<details>\n<summary>MIT: serde 1.0.0</summary>\n\n```text\n<b>Copyright</b>\n```\n\n</details>\n",
    );
    expect(html).not.toContain("<h1");
    expect(html).not.toContain("<a ");
    expect(html).toContain("https://github.com/serde-rs/serde");
    expect(html).toContain("<details>");
    expect(html).toContain("&lt;b&gt;Copyright");
  });

  it("has a translation for every label in the diagrams", () => {
    const diagrams = new Set(imagePaths(manualSource("en")).filter((p) => p.startsWith("diagrams/")));
    for (const path of diagrams) {
      for (const key of diagramLabels(path.slice("diagrams/".length))) {
        for (const lang of LANGS) expect(label(DICTS[lang], key), `${path}: ${lang} ${key}`).toBeDefined();
      }
    }
  });

  it("renders headings with anchors, figures and the version", () => {
    for (const lang of LANGS) {
      const manual = renderManual(lang, DICTS[lang], "9.8.7", false);
      expect(manual.title).not.toBe("");
      expect(manual.sections.map((s) => s.id)).toEqual(sectionIds(manualSource(lang)));
      expect(manual.html).toContain('<h3 id="charts">');
      expect(manual.html).toContain('<figure class="diagram"><svg');
      expect(manual.html).toContain('<figure class="screenshot"><img');
      expect(manual.html).not.toMatch(/\{#|\{version\}|<p>\s*<figure/);
      expect(renderManual(lang, DICTS[lang], "9.8.7", false).html).toEqual(manual.html);
    }
    expect(renderManual("en", en, "9.8.7", false).html).toContain("9.8.7");
  });

  it("puts the language's labels into the diagrams", () => {
    expect(renderManual("de", de, "1", false).html).toContain(`>${de.relay.pulseDuration}</text>`);
    expect(renderManual("cs", cs, "1", true).html).toContain(".dark.webp");
  });
});
