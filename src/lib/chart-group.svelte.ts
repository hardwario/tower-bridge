import { api } from "./api";
import { fill, t } from "./i18n/index.svelte";
import { METRICS, type History, type Metric, type Series } from "./types";

/** How to pan and zoom, with the platform's modifier key. */
export function zoomHint(): string {
  const mac = typeof navigator !== "undefined" && /Mac/i.test(navigator.platform || navigator.userAgent);
  return fill(t().chart.zoomHint, { mod: mac ? "⌘" : t().chart.ctrlKey });
}

/** Narrowest zoom window (seconds). */
const MIN_ZOOM_S = 60;

/**
 * Charts that share a cursor and a visible window (seconds). Without a zoom
 * they show `home`, the selected time span; panning and zooming move through
 * all stored data, within `bounds`.
 */
export class ChartGroup {
  zoom = $state<[number, number] | null>(null);
  /** The selected time span (seconds). */
  home = $state<[number, number] | null>(null);
  /** From the oldest reading to the newest (or the end of the span); the window stays inside. */
  bounds = $state<[number, number] | null>(null);
  /** First and last reading of the data loaded around the window; part of it always stays visible. */
  loaded = $state<[number, number] | null>(null);
  /** True while the pointer is over one of the charts (pauses live refresh). */
  hovering = $state(false);

  constructor(readonly key: string) {}

  setZoom(min: number, max: number) {
    const lo = Math.min(min, max);
    const hi = Math.max(min, max);
    const span = Math.max(hi - lo, MIN_ZOOM_S);
    this.place((lo + hi) / 2 - span / 2, span);
  }

  /** Zoom by `factor` (< 1 in, > 1 out) keeping `center` (seconds) under the pointer. */
  zoomAt(center: number, factor: number) {
    const cur = this.zoom ?? this.home;
    if (!cur) return;
    const [a0, a1] = cur;
    const span = Math.max(MIN_ZOOM_S, (a1 - a0) * factor);
    this.place(center - ((center - a0) / (a1 - a0)) * span, span);
  }

  /** Show [lo, lo + span], kept inside the data. */
  setWindow(lo: number, span: number) {
    this.place(lo, span);
  }

  /** Back to the live time span. */
  reset() {
    this.zoom = null;
  }

  private place(lo: number, span: number) {
    const b = this.bounds;
    let s = span;
    let start = lo;
    if (b) {
      s = Math.min(span, Math.max(b[1] - b[0], MIN_ZOOM_S));
      start = Math.max(b[0], Math.min(lo, b[1] - s));
    }
    // Never pan into a stretch without data (a gap, or before the first
    // reading): keep at least a tenth of the window on loaded data.
    const k = this.loaded;
    if (k) {
      const keep = s * 0.1;
      start = Math.max(k[0] - s + keep, Math.min(start, k[1] - keep));
    }
    this.zoom = [start, start + s];
  }
}

/** The series of `metric` measured on `channel`. */
export function seriesOf(h: History | null, metric: Metric, channel: number): Series | null {
  if (!h) return null;
  const i = h.channels.indexOf(channel);
  return i < 0 ? null : (h[metric][i] ?? null);
}

/** Whether there is any value at index `i` (any metric, any channel). */
export function hasValueAt(h: History, i: number): boolean {
  return METRICS.some((m) => h[m].some((s) => s.avg[i] !== null));
}

/** First and last time with any reading (seconds), or null. */
export function dataExtent(h: History | null): [number, number] | null {
  if (!h) return null;
  const has = (i: number) => hasValueAt(h, i);
  let i0 = 0;
  while (i0 < h.t.length && !has(i0)) i0++;
  let i1 = h.t.length - 1;
  while (i1 >= 0 && !has(i1)) i1--;
  return i0 <= i1 ? [h.t[i0], h.t[i1]] : null;
}

/** Where panning may go: from the oldest reading to the newest, or the end of the span when later. */
export function panBounds(range: { first: number; last: number } | null, h: History): [number, number] {
  const end = h.to / 1000;
  if (!range) return [h.from / 1000, end];
  return [Math.min(range.first / 1000, end), Math.max(range.last / 1000, end)];
}

/**
 * Loads the zoomed or panned window at full resolution, so zooming reveals
 * detail and panning reaches data outside the selected span. Call during
 * component initialisation; `view(base)` returns the loaded data while it
 * overlaps the window, otherwise the base history.
 */
export function zoomDetail(group: ChartGroup, maxPoints: number) {
  // Replaced whole, never changed in place: no deep reactivity needed.
  let detail = $state.raw<History | null>(null);
  let seq = 0;
  let lastFetch = 0;

  $effect(() => {
    const z = group.zoom;
    const mine = ++seq;
    if (!z) {
      detail = null;
      return;
    }
    const span = z[1] - z[0];
    // Still well covered by what is loaded: nothing to do.
    const d = detail;
    if (d && d.from / 1000 <= z[0] - span * 0.2 && d.to / 1000 >= z[1] + span * 0.2 && d.to - d.from < span * 4000) {
      return;
    }
    // A margin on both sides keeps lines running to the edges and covers further panning.
    const pad = span / 2;
    // At most about five loads a second while dragging.
    const wait = Math.max(0, 200 - (Date.now() - lastFetch));
    const timer = setTimeout(async () => {
      lastFetch = Date.now();
      try {
        const from = Math.floor((z[0] - pad) * 1000);
        const to = Math.ceil((z[1] + pad) * 1000) + 1;
        const h = await api.history(from, to, maxPoints * 2);
        if (mine === seq) detail = h;
      } catch (e) {
        console.error(e);
      }
    }, wait);
    return () => clearTimeout(timer);
  });

  return {
    /** The loaded window, if any. */
    get current(): History | null {
      return detail;
    },
    view(base: History | null): History | null {
      const z = group.zoom;
      if (!z || !detail) return base;
      return detail.from / 1000 < z[1] && detail.to / 1000 > z[0] ? detail : base;
    },
  };
}

export interface WindowStats {
  min: number;
  max: number;
  avg: number;
}

/** min / avg / max of one metric on one channel inside the zoom window (or the whole history). */
export function windowStats(
  h: History | null,
  metric: Metric,
  channel: number,
  zoom: [number, number] | null,
): WindowStats | null {
  if (!h) return null;
  const s = seriesOf(h, metric, channel);
  if (!s) return null;
  let min = Infinity;
  let max = -Infinity;
  let sum = 0;
  let n = 0;
  h.t.forEach((t, i) => {
    if (zoom && (t < zoom[0] || t > zoom[1])) return;
    const v = s.avg[i];
    if (v === null) return;
    sum += v;
    n++;
    min = Math.min(min, s.min[i] ?? v);
    max = Math.max(max, s.max[i] ?? v);
  });
  return n ? { min, max, avg: sum / n } : null;
}
