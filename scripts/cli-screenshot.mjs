#!/usr/bin/env node
// Regenerates the README's screenshot of the tower-bridge-cli dashboard
// (docs/images/cli-dashboard.webp and .dark.webp). The dashboard runs on the
// simulated bridge in a pseudo-terminal, and xterm.js in Chrome draws what it
// printed, in a light and a dark terminal. Needs Python 3 for the
// pseudo-terminal (so macOS or Linux) and Google Chrome or Chromium.
//
//   npm run cli:screenshot
//
// CHROME_PATH=/path/to/chrome overrides the browser.

import { spawnSync } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright-core";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const OUT = join(ROOT, "docs/images");
const COLS = 100;
const ROWS = 30;
// Long enough for the simulated CO2 Module to charge its capacitor (a minute)
// and report, and for a reading to carry its value.
const SECONDS = 100;
// Keys pressed on the way, as "seconds:key": switch the relay to TRUE.
const KEYS = "5:t";

// Runs a command in a pseudo-terminal of the given size, presses the given
// keys, copies what it prints to stdout for the given time, then presses q
// and waits for it to end. Only the output before the q is copied: quitting
// clears the screen.
const PTY = `
import fcntl, os, pty, select, signal, struct, sys, termios, time
cols, rows, secs, keys, argv = int(sys.argv[1]), int(sys.argv[2]), float(sys.argv[3]), sys.argv[4], sys.argv[5:]
keys = sorted((float(t), k.encode()) for t, k in (p.split(":", 1) for p in keys.split()))
pid, fd = pty.fork()
if pid == 0:
    fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
    os.execvp(argv[0], argv)
start = time.monotonic()
end = start + secs
while (left := end - time.monotonic()) > 0:
    while keys and time.monotonic() - start >= keys[0][0]:
        os.write(fd, keys.pop(0)[1])
    if keys:
        left = min(left, max(0, start + keys[0][0] - time.monotonic()))
    if select.select([fd], [], [], left)[0]:
        try:
            data = os.read(fd, 65536)
        except OSError:
            break
        if not data:
            break
        sys.stdout.buffer.write(data)
sys.stdout.buffer.flush()
os.write(fd, b"q")
end = time.monotonic() + 10
while time.monotonic() < end:
    if select.select([fd], [], [], 0.2)[0]:
        try:
            os.read(fd, 65536)
        except OSError:
            pass
    if os.waitpid(pid, os.WNOHANG)[0]:
        sys.exit(0)
os.kill(pid, signal.SIGTERM)
os.waitpid(pid, 0)
`;

// The README's code block colours (GitHub's light and dark), with ANSI
// palettes after GitHub's terminal themes. Gray is the title text and the
// status bar behind black text, so it sits between the two in light mode.
const THEMES = {
  light: {
    background: "#f6f8fa",
    foreground: "#1f2328",
    cursor: "#f6f8fa",
    black: "#1f2328",
    red: "#cf222e",
    green: "#1a7f37",
    yellow: "#9a6700",
    blue: "#0969da",
    magenta: "#8250df",
    cyan: "#1b7c83",
    white: "#8c959f",
    brightBlack: "#6e7781",
    brightRed: "#d1242f",
    brightGreen: "#1a7f37",
    brightYellow: "#9a6700",
    brightBlue: "#218bff",
    brightMagenta: "#a475f9",
    brightCyan: "#3192aa",
    brightWhite: "#ffffff",
  },
  dark: {
    background: "#151b23",
    foreground: "#e6edf3",
    cursor: "#151b23",
    black: "#0d1117",
    red: "#ff7b72",
    green: "#3fb950",
    yellow: "#d29922",
    blue: "#58a6ff",
    magenta: "#bc8cff",
    cyan: "#39c5cf",
    white: "#b1bac4",
    brightBlack: "#6e7681",
    brightRed: "#ffa198",
    brightGreen: "#56d364",
    brightYellow: "#e3b341",
    brightBlue: "#79c0ff",
    brightMagenta: "#d2a8ff",
    brightCyan: "#56d4dd",
    brightWhite: "#f0f6fc",
  },
};

function run(cmd, args, options = {}) {
  const r = spawnSync(cmd, args, { cwd: ROOT, maxBuffer: 64 << 20, ...options });
  if (r.error) throw r.error;
  if (r.status !== 0) throw new Error(`${cmd} ${args.join(" ")} exited with ${r.status}`);
  return r.stdout;
}

run("cargo", ["build", "--quiet", "-p", "tower-bridge-cli"], { stdio: "inherit" });
const metadata = JSON.parse(run("cargo", ["metadata", "--format-version", "1", "--no-deps"]).toString());
const cli = join(metadata.target_directory, "debug", "tower-bridge-cli");

// A data folder of its own: nothing touches the real one or a running app.
const dataDir = mkdtempSync(join(tmpdir(), "tower-bridge-cli-screenshot-"));
let screen;
try {
  const pty = [String(COLS), String(ROWS), String(SECONDS), KEYS];
  const command = [cli, "--simulate", "--data-dir", dataDir, "dashboard"];
  // A day of history, so the chart is full from the start.
  const env = { ...process.env, TERM: "xterm-256color", TOWER_BRIDGE_SEED_DAYS: "1" };
  screen = run("python3", ["-I", "-c", PTY, ...pty, ...command], { env });
} finally {
  rmSync(dataDir, { recursive: true, force: true });
}

const browser = await chromium.launch(
  process.env.CHROME_PATH ? { executablePath: process.env.CHROME_PATH } : { channel: "chrome" },
);
try {
  for (const [name, theme] of Object.entries(THEMES)) {
    const page = await browser.newPage();
    const cdp = await page.context().newCDPSession(page);
    // Transparent, so the frame's rounded corners show what is behind them.
    await cdp.send("Emulation.setDefaultBackgroundColorOverride", { color: { r: 0, g: 0, b: 0, a: 0 } });
    await page.setContent(
      `<div id="frame" style="display:inline-block;padding:14px 16px;border-radius:8px;background:${theme.background}"><div id="term"></div></div>`,
    );
    await page.addStyleTag({ path: join(ROOT, "node_modules/@xterm/xterm/css/xterm.css") });
    await page.addScriptTag({ path: join(ROOT, "node_modules/@xterm/xterm/lib/xterm.js") });
    await page.evaluate(
      ([cols, rows, theme, data]) =>
        new Promise((resolve) => {
          const term = new window.Terminal({
            cols,
            rows,
            theme,
            fontFamily: '"SF Mono", Menlo, "DejaVu Sans Mono", monospace',
            fontSize: 14,
            // Bold black would turn grey, and the status bar is black on grey.
            drawBoldTextInBrightColors: false,
            cursorInactiveStyle: "none",
          });
          term.open(document.getElementById("term"));
          const bytes = Uint8Array.from(atob(data), (c) => c.charCodeAt(0));
          term.write(bytes, resolve);
        }),
      [COLS, ROWS, theme, screen.toString("base64")],
    );
    await page.waitForTimeout(300);
    const box = await page.locator("#frame").boundingBox();
    const { data } = await cdp.send("Page.captureScreenshot", {
      format: "webp",
      quality: 90,
      clip: { ...box, scale: 2 },
    });
    const file = join(OUT, `cli-dashboard${name === "dark" ? ".dark" : ""}.webp`);
    await writeFile(file, Buffer.from(data, "base64"));
    console.log(file.slice(ROOT.length + 1));
    await page.close();
  }
} finally {
  await browser.close();
}
