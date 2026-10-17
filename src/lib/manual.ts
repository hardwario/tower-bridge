import { Marked, type Tokens } from "marked";
import type { Lang, Messages } from "./i18n/index.svelte";

/**
 * The user manual, bundled from `docs/manual/` at build time.
 *
 * - `<lang>.md`: one file per language. Section headings carry a language
 *   independent id, as in `## Charts {#charts}`; the ids are the anchors for
 *   links and must be the same in every language. `{version}` is replaced with
 *   the app version.
 * - `images/<lang>/<name>.webp`: screenshots, with a `.dark.webp` twin for the
 *   dark theme (`npm run manual:screenshots` regenerates them).
 * - `diagrams/<name>.svg`: diagrams, inlined so they follow the theme; a `<text>`
 *   with `data-i18n="relay.pulseTrue"` gets that label of the current language.
 */
const SOURCES = import.meta.glob<string>("../../docs/manual/*.md", {
  query: "?raw",
  import: "default",
  eager: true,
});
const IMAGES = import.meta.glob<string>("../../docs/manual/images/*/*.webp", {
  query: "?url",
  import: "default",
  eager: true,
});
const DIAGRAMS = import.meta.glob<string>("../../docs/manual/diagrams/*.svg", {
  query: "?raw",
  import: "default",
  eager: true,
});
const DIR = "../../docs/manual/";

export interface ManualSection {
  id: string;
  title: string;
  level: number;
}

export interface Manual {
  title: string;
  html: string;
  sections: ManualSection[];
}

const HEADING = /^(#{2,3}) (.+?) \{#([a-z0-9-]+)\}$/gm;
const I18N_TEXT = /(<text\b[^>]*\bdata-i18n="([\w.]+)"[^>]*>)[^<]*(<\/text>)/g;

/** Raw Markdown of the manual in `lang`, falling back to English. */
export function manualSource(lang: Lang): string {
  return SOURCES[`${DIR}${lang}.md`] ?? SOURCES[`${DIR}en.md`] ?? "";
}

/** Section ids in document order. */
export function sectionIds(markdown: string): string[] {
  return [...markdown.matchAll(HEADING)].map((m) => m[3]);
}

/** Paths of the images and diagrams a manual refers to. */
export function imagePaths(markdown: string): string[] {
  return [...markdown.matchAll(/!\[[^\]]*\]\(([^)\s]+)/g)].map((m) => m[1]);
}

export function hasImage(path: string): boolean {
  return `${DIR}${path}` in IMAGES || `${DIR}${path}` in DIAGRAMS;
}

/** The label at a dotted path such as `relay.pulseTrue`, if there is one. */
export function label(m: Messages, path: string): string | undefined {
  const value = path.split(".").reduce<unknown>((o, k) => (o as Record<string, unknown> | undefined)?.[k], m);
  return typeof value === "string" ? value : undefined;
}

/** Keys of the labels a diagram takes from the app's translations. */
export function diagramLabels(name: string): string[] {
  return [...(DIAGRAMS[`${DIR}diagrams/${name}`] ?? "").matchAll(I18N_TEXT)].map((m) => m[2]);
}

const escape = (s: string) =>
  s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c] ?? c);

function diagram(path: string, alt: string, m: Messages): string {
  const svg = (DIAGRAMS[`${DIR}${path}`] ?? "")
    .replace(I18N_TEXT, (all, open: string, key: string, close: string) => {
      const text = label(m, key);
      return text === undefined ? all : `${open}${escape(text)}${close}`;
    })
    // Scale with the column; the viewBox keeps the proportions.
    .replace(/(<svg\b[^>]*?) width="\d+" height="\d+"/, "$1")
    .replace(/aria-label="[^"]*"/, `aria-label="${escape(alt)}"`);
  return `<figure class="diagram">${svg}</figure>`;
}

function screenshot(path: string, alt: string, dark: boolean): string {
  const variant = dark ? path.replace(/\.webp$/, ".dark.webp") : path;
  const src = IMAGES[`${DIR}${variant}`] ?? IMAGES[`${DIR}${path}`];
  if (!src) return "";
  // Captured at twice the size, so they stay sharp on high density screens.
  // Not lazy: an image loading while the page scrolls to a chapter would push the chapter away.
  return `<figure class="screenshot"><img src="${src}" srcset="${src} 2x" alt="${escape(alt)}" /></figure>`;
}

export function renderManual(lang: Lang, m: Messages, version: string, dark: boolean): Manual {
  const source = manualSource(lang).replaceAll("{version}", version);
  const sections: ManualSection[] = [];
  const body = source.replace(HEADING, (_, hashes: string, title: string, id: string) => {
    sections.push({ id, title, level: hashes.length });
    return `<h${hashes.length} id="${id}">${escape(title)}</h${hashes.length}>`;
  });
  const title = /^# (.+)$/m.exec(source)?.[1] ?? "";

  const marked = new Marked({
    gfm: true,
    renderer: {
      image({ href, text }: Tokens.Image) {
        return href.startsWith("diagrams/") ? diagram(href, text, m) : screenshot(href, text, dark);
      },
    },
  });
  const html = marked
    .parse(body.replace(/^# .+\n/m, ""), { async: false })
    // A figure on its own line must not sit inside a paragraph.
    .replace(/<p>\s*(<figure[\s\S]*?<\/figure>)\s*<\/p>/g, "$1");
  return { title, html, sections };
}
