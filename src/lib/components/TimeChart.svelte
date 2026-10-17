<script lang="ts">
  /**
   * Interactive time-series chart: average line with min/max band, a dotted
   * line at the window's average, threshold lines, synced crosshair with a value
   * tooltip and the exact time on the x-axis. The visible window is shared
   * through a ChartGroup: drag to pan, Ctrl/Cmd+scroll or pinch to zoom around
   * the pointer, Shift+drag to zoom into a range. Plain
   * scrolling scrolls the page. The window never leaves the data.
   */
  import uPlot from "uplot";
  import { onDestroy, untrack } from "svelte";
  import { zoomHint, type ChartGroup } from "../chart-group.svelte";
  import { fmtAxis, fmtClock, fmtDateTime, fmtNumber } from "../format";
  import { i18n, t } from "../i18n/index.svelte";
  import { bandColor } from "../palette";
  import { chartTheme, theme, type ChartTheme } from "../theme.svelte";
  import type { AlertRule, History, Metric, TemperatureUnit } from "../types";
  import { decimals, toDisplay, unitLabel } from "../units";

  let {
    metric,
    history,
    unit,
    group,
    rule,
    color,
    show = { min: false, avg: true, max: false },
    height = 170,
    compact = false,
  }: {
    metric: Metric;
    history: History;
    unit: TemperatureUnit;
    group: ChartGroup;
    rule?: AlertRule;
    /** Line colour for the current theme. */
    color: string;
    /** Horizontal lines at the window's minimum, average and maximum. */
    show?: { min: boolean; avg: boolean; max: boolean };
    height?: number;
    compact?: boolean;
  } = $props();

  const MIN_SPAN: Record<Metric, number> = { temperature: 2, humidity: 5, illuminance: 20, pressure: 2 };
  const HOUR = 3_600_000;

  let el: HTMLDivElement;
  let tip: HTMLDivElement;
  let pill: HTMLDivElement;
  let plot: uPlot | null = null;
  let width = $state(0);
  let fontsReady = $state(false);
  document.fonts?.ready.then(() => (fontsReady = true));

  function convert(arr: (number | null)[]): (number | null)[] {
    return arr.map((v) => (v === null ? null : toDisplay(metric, v, unit)));
  }

  function chartData(): uPlot.AlignedData {
    const s = history[metric];
    return [history.t, convert(s.min), convert(s.max), convert(s.avg)];
  }

  function xRange(): [number, number] {
    return group.zoom ?? group.home ?? [history.from / 1000, history.to / 1000];
  }

  function thresholds(): number[] {
    if (!rule?.enabled) return [];
    return [rule.min, rule.max].filter((v): v is number => v !== null).map((v) => toDisplay(metric, v, unit));
  }

  const spanMs = (u: uPlot) => ((u.scales.x.max ?? 0) - (u.scales.x.min ?? 0)) * 1000;

  /** Time under the crosshair, as precise as the data: seconds only for sub-minute buckets. */
  function cursorTime(ts: number, span: number): string {
    const sameDay = new Date(ts).toDateString() === new Date().toDateString();
    return span > 12 * HOUR || !sameDay
      ? fmtDateTime(ts, { seconds: history.bucket_ms < 60_000 })
      : fmtClock(ts, history.bucket_ms < 60_000);
  }

  /** Data indices inside the current x scale. */
  function visibleRange(u: uPlot): [number, number] {
    const xs = u.data[0];
    const min = u.scales.x.min ?? -Infinity;
    const max = u.scales.x.max ?? Infinity;
    let i0 = 0;
    while (i0 < xs.length && xs[i0] < min) i0++;
    let i1 = xs.length - 1;
    while (i1 >= 0 && xs[i1] > max) i1--;
    return [i0, i1];
  }

  function drawOverlays(u: uPlot, th: ChartTheme, lines: number[]) {
    const ctx = u.ctx;
    const pr = uPlot.pxRatio;
    const { left, width: w } = u.bbox;
    const [lo, hi] = [u.scales.y.min ?? 0, u.scales.y.max ?? 0];
    ctx.save();

    // Thresholds (dashed: these are limits, not data).
    ctx.strokeStyle = th.critical;
    ctx.lineWidth = 1.5 * pr;
    ctx.setLineDash([5 * pr, 4 * pr]);
    for (const v of lines) {
      if (v < lo || v > hi) continue;
      const y = Math.round(u.valToPos(v, "y", true)) + 0.5;
      ctx.beginPath();
      ctx.moveTo(left, y);
      ctx.lineTo(left + w, y);
      ctx.stroke();
    }

    // Minimum, average and maximum of the visible window.
    const [i0, i1] = visibleRange(u);
    const mins = u.data[1] as (number | null)[];
    const maxs = u.data[2] as (number | null)[];
    const avgs = u.data[3] as (number | null)[];
    let sum = 0;
    let n = 0;
    let min = Infinity;
    let max = -Infinity;
    for (let i = i0; i <= i1; i++) {
      const a = avgs[i];
      if (a === null) continue;
      sum += a;
      n++;
      min = Math.min(min, mins[i] ?? a);
      max = Math.max(max, maxs[i] ?? a);
    }
    if (n === 0) {
      ctx.restore();
      return;
    }
    const level = (v: number, stroke: string, dash: number[], width: number) => {
      const y = Math.round(u.valToPos(v, "y", true)) + 0.5;
      ctx.strokeStyle = stroke;
      ctx.lineWidth = width * pr;
      ctx.setLineDash(dash.map((x) => x * pr));
      ctx.beginPath();
      ctx.moveTo(left, y);
      ctx.lineTo(left + w, y);
      ctx.stroke();
    };
    // Extremes dashed in the line's colour, the average dotted and neutral.
    if (show.min) level(min, color, [6, 3], 1.25);
    if (show.max) level(max, color, [6, 3], 1.25);
    if (show.avg) level(sum / n, th.muted, [2, 3], 1);
    ctx.setLineDash([]);
    ctx.restore();
  }

  function updateCursor(u: uPlot, d: number, label: string) {
    const idx = u.cursor.idx;
    const left = u.cursor.left ?? -1;
    const avg = idx === null || idx === undefined ? null : (u.data[3][idx] as number | null);
    if (idx === null || idx === undefined || avg === null || left < 0) {
      tip.style.display = "none";
      pill.style.display = "none";
      return;
    }
    const lo = u.data[1][idx] as number | null;
    const hi = u.data[2][idx] as number | null;
    const ts = u.data[0][idx] * 1000;

    tip.replaceChildren();
    const v = document.createElement("div");
    v.className = "v";
    v.textContent = `${fmtNumber(avg, d)} ${label}`;
    tip.append(v);
    const [loText, hiText] = [lo, hi].map((x) => (x === null ? null : fmtNumber(x, d)));
    if (loText !== null && hiText !== null && loText !== hiText) {
      const r = document.createElement("div");
      r.className = "r";
      r.textContent = `${loText} – ${hiText}`;
      tip.append(r);
    }
    tip.style.display = "block";
    const x = u.over.offsetLeft + left;
    const tw = tip.offsetWidth;
    tip.style.left = `${x + 12 + tw > width ? x - 12 - tw : x + 12}px`;

    // Exact time on the x-axis under the crosshair.
    pill.textContent = cursorTime(ts, spanMs(u));
    pill.style.display = "block";
    const pw = pill.offsetWidth;
    pill.style.left = `${Math.max(0, Math.min(x - pw / 2, width - pw))}px`;
    pill.style.top = `${u.over.offsetTop + u.over.offsetHeight + 2}px`;
  }

  /** Drag pans (Shift+drag selects a range, see setSelect); used as uPlot's mousedown. */
  function onMouseDown(u: uPlot, e: MouseEvent, select: (e: MouseEvent) => null) {
    if (e.button !== 0) return;
    if (e.shiftKey) {
      select(e);
      return;
    }
    const [a, b] = xRange();
    const width = u.over.clientWidth;
    const x0 = e.clientX;
    let moved = false;
    const move = (ev: MouseEvent) => {
      const dx = ev.clientX - x0;
      if (!moved && Math.abs(dx) < 3) return;
      moved = true;
      u.over.classList.add("panning");
      group.setWindow(a - (dx / width) * (b - a), b - a);
    };
    const up = () => {
      document.removeEventListener("mousemove", move);
      document.removeEventListener("mouseup", up);
      u.over.classList.remove("panning");
    };
    document.addEventListener("mousemove", move);
    document.addEventListener("mouseup", up);
  }

  /** Ctrl/Cmd+scroll and pinch zoom around the pointer, sideways scrolling pans; plain scrolling is the page's. */
  function attachGestures(u: uPlot) {
    const over = u.over;
    const valAt = (clientX: number) => u.posToVal(clientX - over.getBoundingClientRect().left, "x");
    let gesture: number | null = null;

    over.addEventListener(
      "wheel",
      (e) => {
        const px = e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? over.clientHeight : 1;
        const dx = e.deltaX * px;
        const dy = e.deltaY * px;
        if (e.ctrlKey || e.metaKey) {
          // Ctrl/Cmd+wheel, and trackpad pinch (which arrives as Ctrl+wheel).
          e.preventDefault();
          if (gesture === null) group.zoomAt(valAt(e.clientX), Math.exp(Math.max(-50, Math.min(50, dy)) * 0.01));
        } else if (Math.abs(dx) > Math.abs(dy)) {
          e.preventDefault();
          const [a, b] = xRange();
          group.setWindow(a + (dx / over.clientWidth) * (b - a), b - a);
        }
      },
      { passive: false },
    );

    // WebKit reports trackpad pinch as gesture events rather than Ctrl+wheel.
    type Gesture = UIEvent & { scale: number; clientX: number };
    over.addEventListener("gesturestart", (e) => {
      e.preventDefault();
      gesture = 1;
    });
    over.addEventListener("gesturechange", (e) => {
      e.preventDefault();
      const g = e as Gesture;
      if (gesture === null || !g.scale) return;
      group.zoomAt(valAt(g.clientX), gesture / g.scale);
      gesture = g.scale;
    });
    over.addEventListener("gestureend", () => (gesture = null));
  }

  function build() {
    plot?.destroy();
    plot = null;
    if (!el || width <= 0) return;
    const th = chartTheme();
    const d = decimals(metric);
    const label = unitLabel(metric, unit);
    const lines = thresholds();
    const fontFamily = getComputedStyle(document.documentElement).getPropertyValue("--font").trim();
    const axisFont = `${compact ? 10 : 11}px ${fontFamily}`;

    const opts: uPlot.Options = {
      width,
      height,
      padding: compact ? [14, 4, 0, 4] : [14, 10, 0, 2],
      legend: { show: false },
      cursor: {
        y: false,
        sync: { key: group.key },
        drag: { x: true, y: false, setScale: false },
        bind: {
          mousedown: (u, _target, select) => (e) => {
            onMouseDown(u, e, select);
            return null;
          },
        },
        points: {
          size: (_u, i) => (i === 3 ? 8 : 0),
          width: 2,
          stroke: () => th.surface,
          fill: () => color,
        },
      },
      scales: {
        x: { time: true, auto: false, range: (_u, min, max) => [min, max] },
        y: {
          range: (_u, lo, hi) => {
            if (lo === null || hi === null || !Number.isFinite(lo) || !Number.isFinite(hi)) return [0, 1];
            const span = Math.max(hi - lo, MIN_SPAN[metric]);
            const mid = (lo + hi) / 2;
            return [mid - span * 0.68, mid + span * 0.68];
          },
        },
      },
      axes: [
        {
          stroke: th.muted,
          font: axisFont,
          grid: { stroke: th.grid, width: 1 },
          ticks: { show: false },
          size: compact ? 22 : 30,
          space: compact ? 64 : 76,
          values: (u, splits) =>
            splits.map((v) => {
              // Drop labels that would be cut off at the plot's edges.
              const label = fmtAxis(v * 1000, spanMs(u));
              const x = u.valToPos(v, "x");
              const half = label.length * 3.3;
              const w = u.bbox.width / uPlot.pxRatio;
              return x + half > w + 4 || (compact && x - half < -4) ? "" : label;
            }),
        },
        {
          show: !compact,
          stroke: th.muted,
          font: axisFont,
          grid: { stroke: th.grid, width: 1 },
          ticks: { show: false },
          size: 52,
          values: (_u, vals) => vals.map((v) => fmtNumber(v, d === 0 && Math.abs(v) >= 1000 ? 0 : d)),
        },
      ],
      series: [
        {},
        { stroke: "transparent", points: { show: false } },
        { stroke: "transparent", points: { show: false } },
        { stroke: color, width: 2, points: { show: false } },
      ],
      bands: [{ series: [2, 1], fill: bandColor(color, theme.effective === "dark") }],
      hooks: {
        draw: [(u) => drawOverlays(u, th, lines)],
        setCursor: [(u) => updateCursor(u, d, label)],
        setSelect: [
          (u) => {
            if (u.select.width > 6) {
              group.setZoom(u.posToVal(u.select.left, "x"), u.posToVal(u.select.left + u.select.width, "x"));
            }
            u.setSelect({ left: 0, top: 0, width: 0, height: 0 }, false);
          },
        ],
      },
    };
    plot = new uPlot(opts, chartData(), el);
    const [xMin, xMax] = xRange();
    plot.setScale("x", { min: xMin, max: xMax });
    attachGestures(plot);
  }

  // Rebuild when the look changes (unit, thresholds, size, theme, language, formats, fonts).
  $effect(() => {
    void [
      unit,
      rule,
      color,
      show.min,
      show.avg,
      show.max,
      width,
      height,
      compact,
      theme.effective,
      i18n.lang,
      i18n.dateFormat,
      i18n.timeFormat,
      fontsReady,
    ];
    untrack(build);
  });

  // New data or zoom: update in place, keeping the crosshair where it is.
  $effect(() => {
    const data = chartData();
    const [xMin, xMax] = xRange();
    if (!plot) return;
    plot.setData(data);
    plot.setScale("x", { min: xMin, max: xMax });
  });

  onDestroy(() => plot?.destroy());
</script>

<div
  class="chart"
  class:compact
  bind:clientWidth={width}
  style:height="{height}px"
  style:--series={color}
  role="img"
  aria-label="{t().metrics[metric]} ({unitLabel(metric, unit)})"
  title={zoomHint()}
  onpointerenter={() => (group.hovering = true)}
  onpointerleave={() => (group.hovering = false)}
>
  <div bind:this={el}></div>
  <div class="tip" bind:this={tip}></div>
  <div class="pill" bind:this={pill}></div>
</div>

<style>
  .chart {
    position: relative;
    width: 100%;
  }
  .tip {
    display: none;
    position: absolute;
    top: 4px;
    z-index: 2;
    padding: 5px 9px;
    border-radius: 8px;
    background: var(--surface);
    border: 1px solid var(--border-strong);
    box-shadow: 0 4px 14px rgba(0, 0, 0, 0.12);
    pointer-events: none;
    white-space: nowrap;
  }
  .tip :global(.v) {
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }
  .tip :global(.r) {
    font-size: 12px;
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }
  .pill {
    display: none;
    position: absolute;
    z-index: 2;
    padding: 1px 6px;
    border-radius: 5px;
    background: var(--text);
    color: var(--surface);
    font-size: 11px;
    font-weight: 500;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    pointer-events: none;
  }
  .chart :global(.u-over) {
    cursor: crosshair;
  }
  .chart :global(.u-over.panning) {
    cursor: grabbing;
  }
  .chart :global(.u-select) {
    background: color-mix(in srgb, var(--series) 16%, transparent);
  }
</style>
