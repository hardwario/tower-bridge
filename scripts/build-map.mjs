#!/usr/bin/env node
// Builds the offline map of the GPS card, src/assets/map/world.json: one TopoJSON
// topology with the countries, lakes, rivers and populated places of Natural Earth
// at 1:10 million. Run it only to update the data (it downloads about 40 MB):
//
//   npm run map:build
//
// Sources, all free to embed:
// - countries: world-atlas 2.0.2 (ISC), Natural Earth's admin 0 countries as a topology;
// - lakes, rivers, places: Natural Earth 5.1.2 (public domain);
// - Czech and Slovak place names: Wikidata labels (CC0), by the places' Wikidata ids.
//
// Lakes and rivers become arcs of their own in the countries' quantization grid
// (about 0.0036° by 0.0017°). Places are points with their Natural Earth min_zoom
// (the web map zoom from which to label them) and their names, a language's name
// only where it differs from the English one.

import { mkdir, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const OUT = join(ROOT, "src/assets/map/world.json");
const WORLD_ATLAS = "https://cdn.jsdelivr.net/npm/world-atlas@2.0.2/countries-10m.json";
const NATURAL_EARTH = "https://raw.githubusercontent.com/nvkelso/natural-earth-vector/v5.1.2/geojson/";
const WIKIDATA = "https://www.wikidata.org/w/api.php";
const USER_AGENT = "tower-bridge-map-builder/1.0 (https://github.com/hardwario/tower-bridge)";

async function get(url) {
  const res = await fetch(url, { headers: { "User-Agent": USER_AGENT } });
  if (!res.ok) throw new Error(`${url}: HTTP ${res.status}`);
  return res.json();
}

/** Names go through the repository's dash rule like any other text. */
const clean = (s) => (typeof s === "string" ? s.replace(/[\u2013\u2014]/g, "-").trim() : "");

const topology = await get(WORLD_ATLAS);
delete topology.objects.land;
for (const g of topology.objects.countries.geometries) {
  delete g.properties;
  delete g.id;
}
const [kx, ky] = topology.transform.scale;
const [tx, ty] = topology.transform.translate;
const quantize = ([lon, lat]) => [Math.round((lon - tx) / kx), Math.round((lat - ty) / ky)];

/** Adds a line or ring as an arc (delta encoded, repeated points dropped); returns its index or null. */
function addArc(coordinates, minPoints) {
  const points = [];
  for (const c of coordinates) {
    const q = quantize(c);
    const last = points[points.length - 1];
    if (!last || last[0] !== q[0] || last[1] !== q[1]) points.push(q);
  }
  if (points.length < minPoints) return null;
  const arc = points.map((p, i) => (i === 0 ? p : [p[0] - points[i - 1][0], p[1] - points[i - 1][1]]));
  topology.arcs.push(arc);
  return topology.arcs.length - 1;
}

const zoomOf = (props) => Math.round((props.min_zoom ?? props.MIN_ZOOM ?? 10) * 10) / 10;

console.log("lakes");
const lakes = [];
for (const f of (await get(`${NATURAL_EARTH}ne_10m_lakes.geojson`)).features) {
  const polygons = f.geometry.type === "Polygon" ? [f.geometry.coordinates] : f.geometry.coordinates;
  for (const polygon of polygons) {
    const rings = polygon.map((ring) => addArc(ring, 4)).filter((i) => i !== null);
    if (rings.length)
      lakes.push({ type: "Polygon", arcs: rings.map((i) => [i]), properties: { z: zoomOf(f.properties) } });
  }
}

console.log("rivers");
const rivers = [];
for (const f of (await get(`${NATURAL_EARTH}ne_10m_rivers_lake_centerlines.geojson`)).features) {
  // Centerlines through lakes would only draw water on water.
  if (f.properties.featurecla !== "River") continue;
  const lines = f.geometry.type === "LineString" ? [f.geometry.coordinates] : f.geometry.coordinates;
  const arcs = lines.map((line) => addArc(line, 2)).filter((i) => i !== null);
  if (arcs.length)
    rivers.push({ type: "MultiLineString", arcs: arcs.map((i) => [i]), properties: { z: zoomOf(f.properties) } });
}

console.log("places");
const features = (await get(`${NATURAL_EARTH}ne_10m_populated_places.geojson`)).features;
const ids = [...new Set(features.map((f) => f.properties.WIKIDATAID).filter(Boolean))];
const labels = new Map();
for (let i = 0; i < ids.length; i += 50) {
  const batch = ids.slice(i, i + 50).join("|");
  const data = await get(`${WIKIDATA}?action=wbgetentities&ids=${batch}&props=labels&languages=cs|sk&format=json`);
  for (const [id, entity] of Object.entries(data.entities ?? {})) {
    labels.set(id, { cs: entity.labels?.cs?.value, sk: entity.labels?.sk?.value });
  }
  process.stdout.write(`\r  Wikidata ${Math.min(i + 50, ids.length)}/${ids.length}`);
}
process.stdout.write("\n");

const places = features
  .map((f) => {
    const p = f.properties;
    const name = clean(p.NAME_EN) || clean(p.NAME);
    const props = { z: zoomOf(p), n: name };
    const other = { de: p.NAME_DE, pl: p.NAME_PL, ...labels.get(p.WIKIDATAID) };
    for (const lang of ["cs", "sk", "de", "pl"]) {
      const v = clean(other[lang]);
      if (v && v !== name) props[lang] = v;
    }
    return { type: "Point", coordinates: quantize(f.geometry.coordinates), properties: props };
  })
  // The most important first: the map labels in this order and skips what collides.
  .sort((a, b) => a.properties.z - b.properties.z);

topology.objects.lakes = { type: "GeometryCollection", geometries: lakes };
topology.objects.rivers = { type: "GeometryCollection", geometries: rivers };
topology.objects.places = { type: "GeometryCollection", geometries: places };

await mkdir(dirname(OUT), { recursive: true });
const json = JSON.stringify(topology);
await writeFile(OUT, json);
console.log(
  `${OUT}: ${(json.length / 1e6).toFixed(2)} MB, ${topology.arcs.length} arcs, ` +
    `${lakes.length} lakes, ${rivers.length} rivers, ${places.length} places`,
);
