#!/usr/bin/env node
// Smoke test of the whole UI in the browser preview (preview.html, mocked
// backend): every view and device state, in several languages and both
// themes, with a few interactions. Fails on page errors, console errors and
// visible "undefined", "NaN" or unfilled "{placeholder}" text.
//
//   npm run test:ui            in Chrome (Windows renders the app with Chromium)
//   npm run test:ui:webkit     in WebKit (macOS and Linux render the app with it)
//
// Chrome is the installed one (CHROME_PATH=/path/to/chrome overrides it);
// WebKit is Playwright's build: npx playwright-core install webkit.

import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium, webkit } from "playwright-core";
import { createServer } from "vite";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");

/**
 * [query, steps]; a step is ["click" | "hover", selector], ["tab", name] or
 * ["drawn", canvas selector] (waits until the canvas shows more than a few colours).
 */
const CASES = [];
for (const lang of ["en", "cs", "de"]) {
  for (const theme of ["light", "dark"]) {
    const base = `lang=${lang}&theme=${theme}&relay=true`;
    CASES.push([
      base,
      [
        ["hover", "article.metric .u-over"],
        ["click", "article.metric button.maximize"],
        ["click", "article.metric button.maximize"],
        ["click", ".legend .toggle.min"],
        // Hide a line from the legend; the other stays.
        ["click", ".legend button.name"],
        ["click", ".pulse-btn"],
        ["click", ".card.device button.btn:nth-of-type(2)"],
        ["tab", "data"],
        ["click", ".filters button:has-text('1')"],
        // The custom range, with its date fields.
        ["click", ".filters .seg button:last-child"],
        ["tab", "alerts"],
        ["tab", "settings"],
        ["tab", "manual"],
        ["click", ".toc a[href='#mqtt']"],
        ["click", ".screenshot img"],
      ],
    ]);
  }
}
for (const state of ["not_found", "no_module", "permission"]) {
  CASES.push([
    `lang=en&state=${state}`,
    [
      ["tab", "data"],
      ["tab", "settings"],
    ],
  ]);
}
CASES.push([
  "lang=sk&tags=0&relay=unknown",
  [
    ["hover", "article.metric .u-over"],
    ["tab", "data"],
  ],
]);
CASES.push(["lang=pl&gap=2", [["tab", "data"]]]);
// The sidebar folded to its icons, and every tab reached that way.
CASES.push([
  "lang=de&theme=dark&relay=true",
  [["click", ".fold"], ["tab", "data"], ["tab", "alerts"], ["tab", "settings"], ["tab", "manual"], ["click", ".fold"]],
]);
CASES.push(["lang=cs&bridges=2", []]);
// Notifications turned off in macOS: "Send test" says so and offers the settings.
CASES.push([
  "tab=alerts&lang=cs&notify=blocked",
  [
    ["click", "button.btn:has-text('Odeslat')"],
    ["click", "button.btn:has-text('Otevřít nastavení')"],
  ],
]);
// The third-party notices from the manual's About chapter, a license text unfolded, and back.
CASES.push([
  "tab=manual&lang=sk&theme=dark",
  [
    ["click", ".prose a[href='#notices']"],
    ["click", ".notices summary"],
    ["click", ".page-head .btn"],
  ],
]);
CASES.push([
  "lang=de&decimal=point",
  [
    ["tab", "data"],
    ["tab", "settings"],
  ],
]);
CASES.push(["lang=en&nfc=empty", [["click", ".card.nfc input"]]]);
CASES.push([
  "lang=en&co2=1&gps=fix",
  [
    ["drawn", ".card.gps canvas"],
    // Zoom out on the GPS map.
    ["click", ".card.gps .tools button:nth-child(2)"],
    ["hover", "article.metric .u-over"],
    ["tab", "data"],
    ["tab", "alerts"],
    ["tab", "settings"],
  ],
]);
CASES.push([
  "lang=de&gps=fix&tags=0&theme=dark",
  [
    ["drawn", ".card.gps canvas"],
    ["click", ".card.gps .tools button"],
  ],
]);
CASES.push(["lang=cs&co2=1&gps=nofix&theme=dark", [["tab", "data"]]]);
CASES.push(["lang=cs&nfc=url&theme=dark", []]);
CASES.push(["lang=pl&nfc=unformatted", []]);
for (const mqtt of ["off", "connecting", "no_host"]) CASES.push([`lang=de&mqtt=${mqtt}`, [["tab", "settings"]]]);
// A newer version: the Dashboard banner's Download, About with Check now, and the banner closed.
CASES.push([
  "lang=pl&theme=dark&update=available",
  [
    ["click", ".banner .actions .btn.primary"],
    ["tab", "settings"],
    ["click", "button:has-text('Sprawdź teraz')"],
    ["tab", "overview"],
    ["click", ".banner .actions .close"],
  ],
]);
for (const update of ["checking", "failed", "unchecked"]) CASES.push([`lang=de&update=${update}`, [["tab", "settings"]]]);

const server = await createServer({
  root: ROOT,
  logLevel: "error",
  server: { port: 1431, strictPort: false },
});
await server.listen();
const url = server.resolvedUrls?.local[0] ?? "http://localhost:1431/";
const engine = process.argv.includes("--webkit") ? "webkit" : "chrome";
const browser =
  engine === "webkit"
    ? await webkit.launch()
    : await chromium.launch(
        process.env.CHROME_PATH ? { executablePath: process.env.CHROME_PATH } : { channel: "chrome" },
      );
console.log(`UI smoke test in ${engine}`);

const failures = [];
try {
  for (const [query, steps] of CASES) {
    const page = await browser.newPage({
      viewport: { width: 1200, height: 900 },
    });
    const problems = [];
    page.on("pageerror", (e) => problems.push(`page error: ${e.message}`));
    page.on("console", (m) => {
      if (m.type() === "error") problems.push(`console: ${m.text()}`);
    });
    await page.goto(`${url}preview.html?${query}`);
    await page.waitForSelector("nav button[data-tab]");
    await page.waitForTimeout(600);
    const check = async (where) => {
      // Layout: nothing wider than the window, and no squeezed status dots
      // (WebKit once shrank one to a thin bar next to a long label).
      const layout = await page.evaluate(() => {
        const out = [];
        const main = document.querySelector("main");
        if (main && main.scrollWidth > main.clientWidth + 1)
          out.push(`main overflows by ${main.scrollWidth - main.clientWidth}px`);
        for (const dot of document.querySelectorAll(".dot")) {
          const r = dot.getBoundingClientRect();
          if (r.height > 0 && r.width > 0 && (r.width < 5 || Math.abs(r.width - r.height) > 1)) {
            out.push(`a dot is ${r.width.toFixed(1)}×${r.height.toFixed(1)}px`);
          }
        }
        return out;
      });
      for (const l of layout) problems.push(`${where}: ${l}`);
      const text = await page.evaluate(() => document.querySelector("main")?.innerText ?? "");
      for (const bad of [/\bundefined\b/, /\bNaN\b/, /\{[a-z_]+\}/]) {
        const m = text.match(bad);
        // Code blocks in the manual may show placeholders on purpose.
        if (m && !(m[0].startsWith("{") && query.includes("manual"))) problems.push(`${where}: shows "${m[0]}"`);
      }
    };
    await check("start");
    // Several lines in a panel: each colour picker says which line it sets.
    const unnamed = await page.evaluate(
      () =>
        [...document.querySelectorAll(".legend")].filter((legend) => {
          const names = [...legend.querySelectorAll("button[aria-haspopup]")].map((b) => b.getAttribute("aria-label"));
          return new Set(names).size < names.length;
        }).length,
    );
    if (unnamed) problems.push(`${unnamed} legend(s) with colour pickers of the same name`);
    for (const [kind, target] of steps) {
      try {
        if (kind === "tab") await page.click(`nav button[data-tab="${target}"]`);
        else if (kind === "click") await page.locator(target).first().click({ timeout: 2000 });
        else if (kind === "drawn") {
          await page.waitForFunction(
            (sel) => {
              const c = document.querySelector(sel);
              const d = c?.width ? c.getContext("2d").getImageData(0, 0, c.width, c.height).data : [];
              const colours = new Set();
              for (let i = 0; i < d.length; i += 4 * 97) colours.add((d[i] << 16) | (d[i + 1] << 8) | d[i + 2]);
              return colours.size > 4;
            },
            target,
            { timeout: 10000 },
          );
        } else if (kind === "hover") {
          const box = await page.locator(target).first().boundingBox({ timeout: 2000 });
          if (box) await page.mouse.move(box.x + box.width * 0.6, box.y + box.height / 2);
        }
        await page.waitForTimeout(350);
        if (kind === "click" && target.includes("screenshot")) await page.keyboard.press("Escape");
        await check(`${kind} ${target}`);
      } catch (e) {
        problems.push(`${kind} ${target}: ${e.message.split("\n")[0]}`);
      }
    }
    await page.close();
    if (problems.length) failures.push(`${query}\n  ${[...new Set(problems)].join("\n  ")}`);
    else console.log(`ok  ${query}`);
  }
} finally {
  await browser.close();
  await server.close();
}

if (failures.length) {
  console.error(`\n${failures.length} case(s) failed:\n${failures.join("\n")}`);
  process.exit(1);
}
