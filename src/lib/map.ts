/**
 * The offline map of the GPS card: Natural Earth countries, lakes, rivers and
 * places at 1:10 million (src/assets/map/world.json, built by
 * scripts/build-map.mjs), drawn in Web Mercator on a canvas.
 *
 * Coordinates are "world units": the whole Mercator square is 0 to 1 on both
 * axes, so at zoom z one world unit is 256 * 2^z CSS pixels, as on web maps,
 * and the places' Natural Earth min_zoom applies unchanged.
 */
import type { Lang } from "./i18n/index.svelte";

export const MIN_ZOOM = 1;
export const MAX_ZOOM = 10;
const MAX_LAT = 85.0511287798;
const EARTH_CIRCUMFERENCE = 40_075_016.686;

/** Longitude and latitude in degrees to world units. */
export function mercator(lon: number, lat: number): [number, number] {
  const phi = (Math.max(-MAX_LAT, Math.min(MAX_LAT, lat)) * Math.PI) / 180;
  return [(lon + 180) / 360, 0.5 - Math.log(Math.tan(Math.PI / 4 + phi / 2)) / (2 * Math.PI)];
}

/** CSS pixels per world unit. */
export const scaleOf = (zoom: number) => 256 * 2 ** zoom;

type Box = [number, number, number, number];
type Props = Record<string, string | number>;

interface Topology {
  transform: { scale: [number, number]; translate: [number, number] };
  arcs: [number, number][][];
  objects: Record<string, { type: "GeometryCollection"; geometries: Geometry[] }>;
}

type Geometry =
  | { type: "Polygon"; arcs: number[][]; properties?: Props }
  | { type: "MultiPolygon"; arcs: number[][][]; properties?: Props }
  | { type: "LineString"; arcs: number[]; properties?: Props }
  | { type: "MultiLineString"; arcs: number[][]; properties?: Props }
  | { type: "Point"; coordinates: [number, number]; properties?: Props };

/** A polygon (rings) or a set of lines, as arc references: `~i` is arc i reversed. */
export interface Shape {
  parts: number[][];
  box: Box;
  minZoom: number;
}

export interface Place {
  x: number;
  y: number;
  minZoom: number;
  names: Partial<Record<Lang | "n", string>>;
}

export interface MapData {
  /** Every arc's points in world units, x and y interleaved. */
  arcs: Float32Array[];
  boxes: Box[];
  land: Shape[];
  /** Arcs shared by two countries. */
  borders: Shape[];
  lakes: Shape[];
  rivers: Shape[];
  places: Place[];
}

const arcIndex = (ref: number) => (ref < 0 ? ~ref : ref);

function union(boxes: Box[], refs: Iterable<number>): Box {
  const box: Box = [Infinity, Infinity, -Infinity, -Infinity];
  for (const ref of refs) {
    const b = boxes[arcIndex(ref)];
    box[0] = Math.min(box[0], b[0]);
    box[1] = Math.min(box[1], b[1]);
    box[2] = Math.max(box[2], b[2]);
    box[3] = Math.max(box[3], b[3]);
  }
  return box;
}

/** Decode the topology into world units, with a bounding box per arc and shape for culling. */
export function decode(topology: Topology): MapData {
  const [kx, ky] = topology.transform.scale;
  const [tx, ty] = topology.transform.translate;
  const arcs: Float32Array[] = [];
  const boxes: Box[] = [];
  for (const arc of topology.arcs) {
    const points = new Float32Array(arc.length * 2);
    const box: Box = [Infinity, Infinity, -Infinity, -Infinity];
    let qx = 0;
    let qy = 0;
    arc.forEach(([dx, dy], i) => {
      qx += dx;
      qy += dy;
      const [x, y] = mercator(qx * kx + tx, qy * ky + ty);
      points[2 * i] = x;
      points[2 * i + 1] = y;
      box[0] = Math.min(box[0], x);
      box[1] = Math.min(box[1], y);
      box[2] = Math.max(box[2], x);
      box[3] = Math.max(box[3], y);
    });
    arcs.push(points);
    boxes.push(box);
  }

  const zoomOf = (g: Geometry) => Number(g.properties?.z ?? 0);
  const polygons = (g: Geometry): number[][][] =>
    g.type === "Polygon" ? [g.arcs] : g.type === "MultiPolygon" ? g.arcs : [];
  // One shape per polygon, so culling can skip the far islands of a country.
  const areas = (name: string): Shape[] =>
    (topology.objects[name]?.geometries ?? []).flatMap((g) =>
      polygons(g).map((rings) => ({ parts: rings, box: union(boxes, rings.flat()), minZoom: zoomOf(g) })),
    );
  const lines = (name: string): Shape[] =>
    (topology.objects[name]?.geometries ?? []).flatMap((g) => {
      const parts = g.type === "LineString" ? [g.arcs] : g.type === "MultiLineString" ? g.arcs : [];
      return parts.length ? [{ parts, box: union(boxes, parts.flat()), minZoom: zoomOf(g) }] : [];
    });

  // An arc used by two different countries is a border; one used once is coast.
  const owner = new Int32Array(arcs.length).fill(-1);
  const border = new Set<number>();
  (topology.objects.countries?.geometries ?? []).forEach((g, country) => {
    for (const ref of polygons(g).flat(2)) {
      const i = arcIndex(ref);
      if (owner[i] === -1) owner[i] = country;
      else if (owner[i] !== country) border.add(i);
    }
  });

  const places: Place[] = (topology.objects.places?.geometries ?? []).flatMap((g) => {
    if (g.type !== "Point") return [];
    const [x, y] = mercator(g.coordinates[0] * kx + tx, g.coordinates[1] * ky + ty);
    const { z, ...names } = g.properties ?? {};
    return [{ x, y, minZoom: Number(z ?? 0), names: names as Place["names"] }];
  });

  return {
    arcs,
    boxes,
    land: areas("countries"),
    borders: [...border].map((i) => ({ parts: [[i]], box: boxes[i], minZoom: 0 })),
    lakes: areas("lakes"),
    rivers: lines("rivers"),
    places,
  };
}

let loading: Promise<MapData> | null = null;

/** The map, fetched and decoded once (about 7 MB, only when a GPS card shows a position). */
export function loadMap(): Promise<MapData> {
  loading ??= import("../assets/map/world.json?url")
    .then((m) => fetch(m.default))
    .then((r) => {
      if (!r.ok) throw new Error(`map: HTTP ${r.status}`);
      return r.json();
    })
    .then(decode)
    .catch((e) => {
      loading = null;
      throw e;
    });
  return loading;
}

/** What the map shows: its centre in world units, the zoom and its size in CSS pixels. */
export interface View {
  x: number;
  y: number;
  zoom: number;
  width: number;
  height: number;
}

export interface MapTheme {
  water: string;
  land: string;
  border: string;
  river: string;
  text: string;
  halo: string;
  marker: string;
  markerRing: string;
  accuracy: string;
  font: string;
}

export interface Marker {
  x: number;
  y: number;
  /** Horizontal accuracy in metres, drawn as a circle once it is large enough to see. */
  accuracy: number | null;
  latitude: number;
}

/** The world copies in view: the map repeats sideways. */
function offsets(view: View, s: number): number[] {
  const half = view.width / 2 / s;
  const out: number[] = [];
  for (let k = Math.floor(view.x - half); k <= Math.floor(view.x + half); k++) out.push(k);
  return out;
}

/**
 * The parts of the shapes in view as one path, those whose min_zoom is at most
 * `shownFrom`; points closer than a pixel to the last one are skipped.
 */
function trace(data: MapData, shapes: Shape[], view: View, close: boolean, shownFrom = view.zoom): Path2D {
  const s = scaleOf(view.zoom);
  const path = new Path2D();
  const minY = view.y - view.height / 2 / s;
  const maxY = view.y + view.height / 2 / s;
  const cx = view.width / 2 - view.x * s;
  const cy = view.height / 2 - view.y * s;
  for (const k of offsets(view, s)) {
    const minX = view.x - view.width / 2 / s - k;
    const maxX = view.x + view.width / 2 / s - k;
    const ox = cx + k * s;
    for (const shape of shapes) {
      if (shape.minZoom > shownFrom) continue;
      const b = shape.box;
      if (b[0] > maxX || b[2] < minX || b[1] > maxY || b[3] < minY) continue;
      for (const part of shape.parts) {
        let first = true;
        let lx = 0;
        let ly = 0;
        for (const ref of part) {
          const pts = data.arcs[arcIndex(ref)];
          const n = pts.length / 2;
          for (let j = 0; j < n; j++) {
            const i = ref < 0 ? n - 1 - j : j;
            const px = ox + pts[2 * i] * s;
            const py = cy + pts[2 * i + 1] * s;
            if (first) {
              path.moveTo(px, py);
              first = false;
            } else if (Math.abs(px - lx) + Math.abs(py - ly) < 0.8 && j < n - 1) {
              continue;
            } else {
              path.lineTo(px, py);
            }
            lx = px;
            ly = py;
          }
        }
        if (close) path.closePath();
      }
    }
  }
  return path;
}

/**
 * Draw the map (or only the water and the marker while it loads) into a context
 * scaled to CSS pixels. Labels keep out of `reserved` (screen boxes under the
 * map's buttons).
 */
export function drawMap(
  ctx: CanvasRenderingContext2D,
  data: MapData | null,
  view: View,
  theme: MapTheme,
  marker: Marker | null,
  lang: Lang,
  reserved: Box[] = [],
) {
  const s = scaleOf(view.zoom);
  const toScreen = (x: number, y: number): [number, number] => [
    (x - view.x) * s + view.width / 2,
    (y - view.y) * s + view.height / 2,
  ];
  ctx.fillStyle = theme.water;
  ctx.fillRect(0, 0, view.width, view.height);
  ctx.lineJoin = "round";
  ctx.lineCap = "round";

  if (data) {
    ctx.fillStyle = theme.land;
    ctx.fill(trace(data, data.land, view, true), "evenodd");
    // Lakes and rivers a level before Natural Earth's own zoom: the card is small.
    ctx.fillStyle = theme.water;
    ctx.fill(trace(data, data.lakes, view, true, view.zoom + 1), "evenodd");
    ctx.strokeStyle = theme.river;
    ctx.lineWidth = view.zoom >= 7 ? 1.4 : 1;
    ctx.stroke(trace(data, data.rivers, view, false, view.zoom + 1));
    ctx.strokeStyle = theme.border;
    ctx.lineWidth = view.zoom >= 5 ? 1 : 0.6;
    ctx.stroke(trace(data, data.borders, view, false));
  }

  if (marker && marker.accuracy !== null) {
    const metresPerUnit = EARTH_CIRCUMFERENCE * Math.cos((marker.latitude * Math.PI) / 180);
    const r = (marker.accuracy / metresPerUnit) * s;
    if (r > 8) {
      const [mx, my] = toScreen(marker.x, marker.y);
      ctx.beginPath();
      ctx.arc(mx, my, r, 0, 2 * Math.PI);
      ctx.fillStyle = theme.accuracy;
      ctx.fill();
    }
  }

  if (data) drawPlaces(ctx, data, view, theme, marker, lang, toScreen, reserved);

  if (marker) {
    const [mx, my] = toScreen(marker.x, marker.y);
    ctx.beginPath();
    ctx.arc(mx, my, 6, 0, 2 * Math.PI);
    ctx.fillStyle = theme.marker;
    ctx.fill();
    ctx.lineWidth = 2;
    ctx.strokeStyle = theme.markerRing;
    ctx.stroke();
  }
}

function drawPlaces(
  ctx: CanvasRenderingContext2D,
  data: MapData,
  view: View,
  theme: MapTheme,
  marker: Marker | null,
  lang: Lang,
  toScreen: (x: number, y: number) => [number, number],
  reserved: Box[],
) {
  const s = scaleOf(view.zoom);
  ctx.font = `11px ${theme.font}`;
  ctx.textBaseline = "middle";
  ctx.lineWidth = 3;
  // Space already taken: labels never overlap each other, the marker or the buttons.
  const taken: Box[] = [...reserved];
  const [mx, my] = marker ? toScreen(marker.x, marker.y) : [NaN, NaN];
  if (marker) taken.push([mx - 9, my - 9, mx + 9, my + 9]);
  const free = (b: Box) =>
    b[0] >= 2 && b[2] <= view.width - 2 && taken.every((t) => b[0] > t[2] || b[2] < t[0] || b[1] > t[3] || b[3] < t[1]);
  const label = (name: string, x: number, y: number) => {
    ctx.strokeStyle = theme.halo;
    ctx.strokeText(name, x, y);
    ctx.fillStyle = theme.text;
    ctx.fillText(name, x, y);
  };
  let shown = 0;
  for (const place of data.places) {
    // Sorted by min_zoom, so the rest are for closer views.
    if (place.minZoom > view.zoom || shown >= 60) break;
    for (const k of offsets(view, s)) {
      const [px, py] = toScreen(place.x + k, place.y);
      if (px < 4 || px > view.width - 4 || py < 9 || py > view.height - 9) continue;
      const name = place.names[lang] ?? place.names.n ?? "";
      const w = ctx.measureText(name).width;
      // The place under the marker: its name beside the marker, without a dot.
      if (Math.hypot(px - mx, py - my) < 9) {
        const box: Box = [mx + 10, my - 7, mx + 13 + w, my + 7];
        if (!free(box)) continue;
        taken.push(box);
        shown++;
        label(name, mx + 11, my);
        continue;
      }
      const right: Box = [px - 3, py - 7, px + 7 + w, py + 7];
      const left: Box = [px - 7 - w, py - 7, px + 3, py + 7];
      const box = free(right) ? right : free(left) ? left : null;
      if (!box) continue;
      taken.push(box);
      shown++;
      ctx.beginPath();
      ctx.arc(px, py, 2.5, 0, 2 * Math.PI);
      ctx.fillStyle = theme.text;
      ctx.fill();
      label(name, box === right ? px + 6 : px - 6 - w, py);
    }
  }
}
