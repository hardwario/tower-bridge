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

## Logo

`src/assets/hardwario-tower.svg` is the HARDWARIO TOWER logo
(`hardwario-tower-tower-vertical.svg` from the brand artwork), optimized with
svgo 4 and [`svgo.config.mjs`](svgo.config.mjs):

```sh
svgo --config scripts/svgo.config.mjs -i hardwario-tower-tower-vertical.svg -o src/assets/hardwario-tower.svg
sed 's/fill="#6b6a6a"/fill="#fff"/' src/assets/hardwario-tower.svg > src/assets/hardwario-tower-reversed.svg
```

The config runs several passes but leaves out `convertToQ`, which bent the
outlines by up to 0.08 viewBox units; with it off they stay within 0.007 of
the original (13.3 KB to 7.7 KB). The reversed file, for dark mode, has white
where the logo is grey, like `hardwario-symbol-reversed.svg`.
