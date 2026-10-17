#!/usr/bin/env node
// Regenerates the screenshots of the user manual (docs/manual/images/<lang>/*.webp)
// from the browser preview (preview.html, mocked backend), in every language and
// in the light and dark theme. Needs Google Chrome or Chromium installed.
//
//   npm run manual:screenshots            all languages
//   npm run manual:screenshots -- cs de   only some
//
// CHROME_PATH=/path/to/chrome overrides the browser.

import { mkdir, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright-core";
import { createServer } from "vite";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const OUT = join(ROOT, "docs/manual/images");
const LANGS = ["en", "cs", "sk", "de", "pl"];
const THEMES = ["light", "dark"];
const VIEWPORT = { width: 1200, height: 780 };

/** What to capture: a whole tab, or one element of it (optionally in a narrower window). */
const SHOTS = [
  { name: "dashboard", tab: "overview" },
  { name: "relay", tab: "overview", element: ".card.relay" },
  { name: "chart", tab: "overview", element: "article.metric" },
  { name: "data", tab: "data" },
  {
    name: "alerts",
    tab: "alerts",
    element: "main .view > section",
    width: 900,
  },
];

const langs = process.argv.slice(2).filter((a) => LANGS.includes(a));

const server = await createServer({
  root: ROOT,
  logLevel: "error",
  server: { port: 1430, strictPort: false },
});
await server.listen();
const base = server.resolvedUrls?.local[0] ?? "http://localhost:1430/";

const browser = await chromium.launch(
  process.env.CHROME_PATH ? { executablePath: process.env.CHROME_PATH } : { channel: "chrome" },
);

try {
  for (const lang of langs.length ? langs : LANGS) {
    await mkdir(join(OUT, lang), { recursive: true });
    for (const theme of THEMES) {
      const context = await browser.newContext({
        viewport: VIEWPORT,
        deviceScaleFactor: 1,
        colorScheme: theme,
      });
      const page = await context.newPage();
      const cdp = await context.newCDPSession(page);
      await page.goto(`${base}preview.html?lang=${lang}&theme=${theme}&relay=true`);
      await page.waitForSelector(".card.relay");
      await page.waitForTimeout(1200);

      for (const shot of SHOTS) {
        await page.setViewportSize({
          width: shot.width ?? VIEWPORT.width,
          height: VIEWPORT.height,
        });
        await page.click(`nav button[data-tab="${shot.tab}"]`);
        await page.mouse.move(0, VIEWPORT.height - 1);
        await page.waitForTimeout(700);
        const box = shot.element ? await page.locator(shot.element).first().boundingBox() : { x: 0, y: 0, ...VIEWPORT };
        if (!box) throw new Error(`${shot.name}: ${shot.element} not found`);
        // WebP straight from Chrome keeps the files small without extra tools.
        const { data } = await cdp.send("Page.captureScreenshot", {
          format: "webp",
          quality: 82,
          clip: {
            x: box.x,
            y: box.y,
            width: box.width,
            height: box.height,
            scale: 2,
          },
        });
        const file = join(OUT, lang, `${shot.name}${theme === "dark" ? ".dark" : ""}.webp`);
        await writeFile(file, Buffer.from(data, "base64"));
        console.log(file.slice(ROOT.length + 1));
      }
      await context.close();
    }
  }
} finally {
  await browser.close();
  await server.close();
}
