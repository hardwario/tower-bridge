# Scripts

- `check-version.mjs`: used by the release workflow to check that the git tag,
  `package.json` and `src-tauri/Cargo.toml` carry the same version.
- `ui-smoke.mjs` (`npm run test:ui`): opens every view and device state of
  the browser preview in several languages and both themes, clicks through
  the main controls and fails on page errors, console errors or visible
  `undefined`, `NaN` or unfilled `{placeholder}` text, horizontal overflow
  and squeezed status dots. `npm run test:ui:webkit` runs it in WebKit, the
  engine macOS and Linux render the app with (install it once with
  `npx playwright-core install webkit`). Both run in CI.
- `manual-screenshots.mjs` (`npm run manual:screenshots`): renders the
  screenshots of the user manual (`docs/manual/images/<lang>/*.webp`, light and
  dark) from the browser preview with Playwright and the installed Chrome.

## Icons

`src-tauri/icons/app-icon.png` and `src-tauri/icons/tray/*.png` were rendered
from the HARDWARIO brand symbol (`artwork/brand/hardwario-symbol/*.svg`):

- `app-icon.png`: 1024 px, white rounded square (824 px, radius 186) with the
  colour symbol 540 px tall. Platform icons are generated from it with
  `npx tauri icon src-tauri/icons/app-icon.png -o src-tauri/icons`.
- `mac-template*.png`: 44 px, black symbol 33 px tall, centred 2 px above the
  middle so it lines up with the menu bar title's digits (macOS template image;
  the `-off` variant at 40 % opacity is shown while disconnected).
- `tile-<size>.png` / `tile-off-<size>.png`: brand red `#E4002B` (grey `#6B6A6A`
  when disconnected) rounded square, radius 22 %, white symbol 72 % tall.
