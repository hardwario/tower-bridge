<script lang="ts">
  import { i18n, t } from "../i18n/index.svelte";
  import {
    MAX_ZOOM,
    MIN_ZOOM,
    clampY,
    drawMap,
    loadMap,
    mercator,
    scaleOf,
    type MapData,
    type MapTheme,
  } from "../map";
  import Icon from "./Icon.svelte";

  let {
    latitude,
    longitude,
    accuracy,
  }: { latitude: number; longitude: number; accuracy: number | null } = $props();

  const ZOOM_KEY = "tower-bridge.gps.zoom";
  const DEFAULT_ZOOM = 7;

  function savedZoom(): number {
    try {
      const z = Number(localStorage.getItem(ZOOM_KEY));
      return z >= MIN_ZOOM && z <= MAX_ZOOM ? z : DEFAULT_ZOOM;
    } catch {
      return DEFAULT_ZOOM;
    }
  }

  const m = $derived(t().gps);
  let box: HTMLDivElement;
  let canvas: HTMLCanvasElement;
  let width = $state(0);
  let height = $state(0);
  let data = $state.raw<MapData | null>(null);
  let failed = $state(false);
  let zoom = $state(savedZoom());
  /** Where the user moved the map; null follows the position. */
  let pan = $state.raw<[number, number] | null>(null);
  /** Bumped when the theme changes, so the colours are read again. */
  let themeTick = $state(0);

  const target = $derived(mercator(longitude, latitude));
  const center = $derived.by((): [number, number] => {
    const [x, y] = pan ?? target;
    return [x, clampY(y, zoom, height)];
  });

  $effect(() => {
    loadMap().then(
      (d) => (data = d),
      () => (failed = true),
    );
  });

  $effect(() => {
    const resize = new ResizeObserver(() => {
      width = box.clientWidth;
      height = box.clientHeight;
    });
    resize.observe(box);
    const media = matchMedia("(prefers-color-scheme: dark)");
    const bump = () => themeTick++;
    media.addEventListener("change", bump);
    const attrs = new MutationObserver(bump);
    attrs.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
    return () => {
      resize.disconnect();
      media.removeEventListener("change", bump);
      attrs.disconnect();
    };
  });

  function theme(): MapTheme {
    const css = getComputedStyle(canvas);
    const v = (name: string) => css.getPropertyValue(name).trim();
    return {
      water: v("--map-water"),
      land: v("--map-land"),
      border: v("--map-border"),
      river: v("--map-river"),
      text: v("--map-text"),
      halo: v("--map-halo"),
      marker: v("--accent"),
      markerRing: v("--surface"),
      accuracy: v("--map-accuracy"),
      font: v("--font"),
    };
  }

  let frame = 0;
  $effect(() => {
    // Everything the drawing depends on, read here so the effect follows it.
    const view = { x: center[0], y: center[1], zoom, width, height };
    const d = data;
    const lang = i18n.lang;
    const marker = { x: target[0], y: target[1], accuracy, latitude };
    const moved = pan !== null;
    void themeTick;
    if (width <= 0 || height <= 0) return;
    cancelAnimationFrame(frame);
    frame = requestAnimationFrame(() => {
      const dpr = window.devicePixelRatio || 1;
      canvas.width = Math.round(width * dpr);
      canvas.height = Math.round(height * dpr);
      const ctx = canvas.getContext("2d");
      if (!ctx) return;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      // Keep labels from under the buttons (top right) and the credit (bottom right).
      const tools: [number, number, number, number] = [width - 44, 0, width, moved ? 112 : 76];
      const credit: [number, number, number, number] = [width - 84, height - 18, width, height];
      drawMap(ctx, d, view, theme(), marker, lang, [tools, credit]);
    });
    return () => cancelAnimationFrame(frame);
  });

  function setZoom(z: number) {
    zoom = Math.max(MIN_ZOOM, Math.min(MAX_ZOOM, z));
    try {
      localStorage.setItem(ZOOM_KEY, String(Math.round(zoom)));
    } catch {
      // Private mode: the zoom is not remembered.
    }
  }

  /** Zoom keeping the point under the pointer in place. */
  function zoomAt(clientX: number, clientY: number, z: number) {
    const r = canvas.getBoundingClientRect();
    const dx = clientX - r.left - width / 2;
    const dy = clientY - r.top - height / 2;
    const before = scaleOf(zoom);
    const [cx, cy] = center;
    setZoom(z);
    const after = scaleOf(zoom);
    moveTo(cx + dx / before - dx / after, cy + dy / before - dy / after);
  }

  function moveTo(x: number, y: number) {
    pan = [x, clampY(y, zoom, height)];
  }

  let drag: { x: number; y: number; cx: number; cy: number } | null = null;

  function pointerdown(e: PointerEvent) {
    if (e.button !== 0) return;
    canvas.setPointerCapture(e.pointerId);
    drag = { x: e.clientX, y: e.clientY, cx: center[0], cy: center[1] };
  }

  function pointermove(e: PointerEvent) {
    if (!drag) return;
    const s = scaleOf(zoom);
    const dx = e.clientX - drag.x;
    const dy = e.clientY - drag.y;
    // A click is not a move: the map keeps following the position.
    if (pan === null && Math.abs(dx) + Math.abs(dy) < 3) return;
    moveTo(drag.cx - dx / s, drag.cy - dy / s);
  }

  function pointerup() {
    drag = null;
  }

  // Ctrl/Cmd+scroll and pinch zoom, as in the charts; plain scrolling scrolls the page.
  $effect(() => {
    const wheel = (e: WheelEvent) => {
      if (!(e.ctrlKey || e.metaKey)) return;
      e.preventDefault();
      const dy = e.deltaMode === 1 ? e.deltaY * 16 : e.deltaY;
      zoomAt(e.clientX, e.clientY, zoom - Math.max(-50, Math.min(50, dy)) * 0.01);
    };
    // WebKit reports trackpad pinch as gesture events rather than Ctrl+wheel.
    type Gesture = UIEvent & { scale: number; clientX: number; clientY: number };
    let start: number | null = null;
    const gesturestart = (e: Event) => {
      e.preventDefault();
      start = zoom;
    };
    const gesturechange = (e: Event) => {
      e.preventDefault();
      const g = e as Gesture;
      if (start !== null && g.scale) zoomAt(g.clientX, g.clientY, start + Math.log2(g.scale));
    };
    const gestureend = () => (start = null);
    canvas.addEventListener("wheel", wheel, { passive: false });
    canvas.addEventListener("gesturestart", gesturestart);
    canvas.addEventListener("gesturechange", gesturechange);
    canvas.addEventListener("gestureend", gestureend);
    return () => {
      canvas.removeEventListener("wheel", wheel);
      canvas.removeEventListener("gesturestart", gesturestart);
      canvas.removeEventListener("gesturechange", gesturechange);
      canvas.removeEventListener("gestureend", gestureend);
    };
  });
</script>

<div class="map" bind:this={box}>
  <canvas
    bind:this={canvas}
    class:moved={pan !== null}
    aria-label={m.mapLabel}
    onpointerdown={pointerdown}
    onpointermove={pointermove}
    onpointerup={pointerup}
    onpointercancel={pointerup}
    ondblclick={(e) => zoomAt(e.clientX, e.clientY, Math.round(zoom) + 1)}
  ></canvas>
  <div class="tools">
    <button
      type="button"
      title={m.zoomIn}
      aria-label={m.zoomIn}
      disabled={zoom >= MAX_ZOOM}
      onclick={() => setZoom(Math.round(zoom) + 1)}
    >
      <Icon name="plus" size={15} />
    </button>
    <button
      type="button"
      title={m.zoomOut}
      aria-label={m.zoomOut}
      disabled={zoom <= MIN_ZOOM}
      onclick={() => setZoom(Math.round(zoom) - 1)}
    >
      <Icon name="minus" size={15} />
    </button>
    {#if pan !== null}
      <button type="button" title={m.recenter} aria-label={m.recenter} onclick={() => (pan = null)}>
        <Icon name="locate" size={15} />
      </button>
    {/if}
  </div>
  <span class="credit">{failed ? m.mapFailed : "Natural Earth"}</span>
</div>

<style>
  .map {
    position: relative;
    height: 240px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    overflow: hidden;
    background: var(--map-water);
  }
  canvas {
    display: block;
    width: 100%;
    height: 100%;
    cursor: grab;
    touch-action: none;
  }
  canvas:active {
    cursor: grabbing;
  }
  .tools {
    position: absolute;
    top: 8px;
    right: 8px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .tools button {
    display: grid;
    place-items: center;
    width: 28px;
    height: 28px;
    padding: 0;
    border: 1px solid var(--border-strong);
    border-radius: 6px;
    background: var(--surface);
    color: var(--text-2);
    cursor: pointer;
  }
  .tools button:hover:not(:disabled) {
    color: var(--text);
    background: var(--surface-2);
  }
  .tools button:disabled {
    opacity: 0.45;
    cursor: default;
  }
  .credit {
    position: absolute;
    right: 6px;
    bottom: 4px;
    font-size: 10.5px;
    color: var(--map-text);
    opacity: 0.75;
    pointer-events: none;
  }
</style>
