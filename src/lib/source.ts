import { fill, t } from "./i18n/index.svelte";
import type {
  BusWarning,
  DeviceInfo,
  DeviceState,
  GpsInfo,
  History,
  Metric,
  ModuleKind,
  NfcInfo,
  Reading,
  RelayInfo,
  SensorSource,
} from "./types";
import { compareLines, historyKeys, type LineKey, lineKey, parseLine, readingKey, sourceKey } from "./series";

/** Product names; they stay the same in every language. */
export const MODULE_NAMES: Record<ModuleKind, string> = {
  climate_module_r1: "Climate Module R1",
  climate_module_r2: "Climate Module R2",
  humidity_tag: "Humidity Tag",
  temperature_tag: "Temperature Tag",
  lux_meter_tag: "Lux Meter Tag",
  barometer_tag: "Barometer Tag",
  co2_module: "CO2 Module",
  voc_lp_tag: "VOC-LP Tag",
};

/** Where a value comes from: module, bus, switch channel and sensor address. */
export interface Source {
  module: string;
  backend: DeviceInfo["backend"];
  channel: number;
  address: number;
  part: string;
}

export const hexAddress = (a: number) => `0x${a.toString(16).toUpperCase().padStart(2, "0")}`;

/** "I²C (chip FT260) → channel 0 → address 0x48 (chip TMP112)" */
export function sourcePath(s: Source): string {
  const c = t().common;
  const bus = s.backend === "hid" ? fill(c.chip, { name: "FT260" }) : "i2c-dev";
  const channel = fill(c.channel, { n: s.channel });
  const address = fill(c.address, { address: hexAddress(s.address) });
  return `I²C (${bus}) → ${channel} → ${address} (${fill(c.chip, { name: s.part })})`;
}

/** "TOWER Climate Module R1 · I²C (chip FT260) → channel 0 → address 0x48 (chip TMP112)" */
export function sourceText(s: Source): string {
  return `${s.module} · ${sourcePath(s)}`;
}

/** Suspected address collisions while the bridge works, otherwise none. */
export const warningsOf = (device: DeviceState): BusWarning[] =>
  device.status === "ready" ? (device.warnings ?? []) : [];

/**
 * "Climate Module R2 MPL3115A2 at 0x60 on channel 0 gives garbled values: …
 * Move the tag to channel 1."
 */
export function collisionText(w: BusWarning): string {
  return fill(t().banners.collisionBody, {
    module: MODULE_NAMES[w.module] ?? w.module,
    sensor: w.sensor,
    address: hexAddress(w.address),
    channel: w.channel,
    other: w.channel === 0 ? 1 : 0,
  });
}

/** Sensor sources while the bridge works, otherwise none. */
export const sourcesOf = (device: DeviceState): SensorSource[] =>
  device.status === "ready" ? (device.sources ?? []) : [];

/** The source of `metric` on a line: of the channel, or a further sensor on it. */
function sourceOf(device: DeviceState, metric: Metric, key: LineKey): SensorSource | undefined {
  return sourcesOf(device).find((x) => x.metric === metric && sourceKey(x) === key);
}

/** The sensor measuring `metric` on line `key` (a channel, or a further sensor on it). */
export function metricSource(device: DeviceState, metric: Metric, key: LineKey): Source | null {
  if (device.status !== "ready") return null;
  const s = sourceOf(device, metric, key);
  if (!s) return null;
  return {
    module: productName(s.module),
    backend: device.device.backend,
    channel: s.channel,
    address: s.address,
    part: s.sensor,
  };
}

/** Names of the modules on each channel, in channel order. */
export function modulesByChannel(sources: SensorSource[]): { channel: number; names: string[] }[] {
  const out: { channel: number; names: string[] }[] = [];
  for (const s of [...sources].sort((a, b) => a.channel - b.channel)) {
    const name = MODULE_NAMES[s.module] ?? s.module;
    const entry = out.find((e) => e.channel === s.channel);
    if (!entry) out.push({ channel: s.channel, names: [name] });
    else if (!entry.names.includes(name)) entry.names.push(name);
  }
  return out;
}

/** A module by its full product name; the revision apart, to be set smaller. */
export interface ModuleLine {
  channel: number;
  product: string;
  revision: string | null;
}

const PRODUCTS: Record<ModuleKind, Omit<ModuleLine, "channel">> = {
  climate_module_r1: { product: "TOWER Climate Module", revision: "R1" },
  climate_module_r2: { product: "TOWER Climate Module", revision: "R2" },
  humidity_tag: { product: "TOWER Humidity Tag", revision: null },
  temperature_tag: { product: "TOWER Temperature Tag", revision: null },
  lux_meter_tag: { product: "TOWER Lux Meter Tag", revision: null },
  barometer_tag: { product: "TOWER Barometer Tag", revision: null },
  co2_module: { product: "TOWER CO2 Module", revision: null },
  voc_lp_tag: { product: "TOWER VOC-LP Tag", revision: null },
};

/** The full product name with its revision, e.g. "TOWER Climate Module R1". */
export function productName(kind: ModuleKind): string {
  const p = PRODUCTS[kind];
  return p ? (p.revision ? `${p.product} ${p.revision}` : p.product) : kind;
}

/** Product names of the modules that are not sensor sources. */
export const RELAY_PRODUCT = "TOWER Relay Module";
export const NFC_PRODUCT = "TOWER NFC Tag";
export const GPS_PRODUCT = "TOWER GPS Module";

/**
 * Every module once, by channel and A to Z within a channel (the user,
 * 2026-10-18). The Relay Module, NFC Tag and GPS Module measure nothing, so
 * they are not among the sources and come as arguments.
 */
export function moduleLines(
  sources: SensorSource[],
  relay: RelayInfo | null,
  nfc: NfcInfo | null = null,
  gps: GpsInfo | null = null,
): ModuleLine[] {
  const out: ModuleLine[] = [];
  for (const s of sources) {
    const line = { channel: s.channel, ...(PRODUCTS[s.module] ?? { product: s.module, revision: null }) };
    if (!out.some((l) => l.channel === line.channel && l.product === line.product)) out.push(line);
  }
  if (relay) out.push({ channel: relay.channel, product: RELAY_PRODUCT, revision: null });
  if (nfc) out.push({ channel: nfc.channel, product: NFC_PRODUCT, revision: null });
  if (gps) out.push({ channel: gps.channel, product: GPS_PRODUCT, revision: null });
  return out.sort((a, b) => a.channel - b.channel || a.product.localeCompare(b.product));
}

/** "Climate Module R1 (channel 0), Humidity Tag, Lux Meter Tag (channel 1)" */
export function describeModules(sources: SensorSource[]): string {
  return modulesByChannel(sources)
    .map((e) => `${e.names.join(", ")} (${fill(t().common.channel, { n: e.channel })})`)
    .join(", ");
}

/**
 * Lines with `metric`: each channel and each further sensor on one (a
 * Humidity Tag next to a Climate Module) measured now, in the latest
 * readings or with data in one of the histories (pass the whole span as well
 * as a zoomed window, so a panel keeps its lines while zooming). Channel 0
 * when there is nothing at all, so a panel still has its one line, unless
 * `fallback` is off.
 */
export function metricLines(
  device: DeviceState,
  metric: Metric,
  histories: History | null | (History | null)[],
  latest: Reading[],
  fallback = true,
): LineKey[] {
  const set = new Set<LineKey>();
  for (const s of sourcesOf(device)) if (s.metric === metric) set.add(sourceKey(s));
  for (const r of latest) if (r[metric] !== null) set.add(readingKey(r));
  for (const h of Array.isArray(histories) ? histories : [histories]) {
    if (!h) continue;
    historyKeys(h).forEach((key, i) => {
      if (h[metric][i]?.avg.some((v) => v !== null)) set.add(key);
    });
  }
  const list = [...set].sort(compareLines);
  return list.length || !fallback ? list : [lineKey(0)];
}

/**
 * Names for the lines of one panel, in the same order: the module, with the
 * channel ("Humidity Tag · channel 1") when a line of another channel would
 * read the same, and with the sensor ("Lux Meter Tag · opt3001-45") when
 * another line of the same channel would.
 */
export function lineLabels(names: { key: LineKey; name: string }[]): string[] {
  return names.map((n) => {
    const me = parseLine(n.key);
    const twins = names.filter((x) => x.name === n.name && x.key !== n.key).map((x) => parseLine(x.key));
    const parts = [n.name];
    if (twins.some((x) => x.channel !== me.channel)) parts.push(fill(t().common.channel, { n: me.channel }));
    if (me.sensor && twins.some((x) => x.channel === me.channel)) parts.push(me.sensor);
    return parts.join(" · ");
  });
}

const KNOWN_KEY = "tower-bridge.known-sources";

/** The browser's storage; none in tests and the like. */
const storage = (): Storage | null => (typeof window === "undefined" ? null : window.localStorage);

/**
 * The module last seen measuring each metric on each line ("temperature:0",
 * "temperature:0/sht20-40").
 */
let known: Record<string, ModuleKind> = (() => {
  try {
    return JSON.parse(storage()?.getItem(KNOWN_KEY) ?? "{}");
  } catch {
    return {};
  }
})();

/** Remember which module measures what, so stored data keeps a name after it is unplugged. */
export function rememberSources(sources: SensorSource[]) {
  let changed = false;
  for (const s of sources) {
    const key = `${s.metric}:${sourceKey(s)}`;
    if (known[key] !== s.module) {
      known = { ...known, [key]: s.module };
      changed = true;
    }
  }
  if (!changed) return;
  try {
    storage()?.setItem(KNOWN_KEY, JSON.stringify(known));
  } catch {
    // Private mode: names are remembered for this session only.
  }
}

/**
 * Which module measures `metric` on line `key`, e.g. "Humidity Tag"; for
 * stored data of a module unplugged since, "Climate Module R1
 * (disconnected)", or "I²C channel 1 (disconnected)" when it was never seen
 * here ("I²C channel 0 · sht20-40 (disconnected)" for a further sensor).
 */
export function lineName(device: DeviceState, metric: Metric, key: LineKey): string {
  const s = sourceOf(device, metric, key);
  if (s) return MODULE_NAMES[s.module] ?? s.module;
  const gone = known[`${metric}:${key}`];
  const { channel, sensor } = parseLine(key);
  const place = fill(t().overview.i2cChannel, { n: channel });
  const name = gone ? (MODULE_NAMES[gone] ?? gone) : sensor ? `${place} · ${sensor}` : place;
  return fill(t().chart.disconnected, { name });
}

/** `lineName`, told apart from the panel's other lines of the same name (see `lineLabels`). */
export function lineLabel(
  device: DeviceState,
  metric: Metric,
  key: LineKey,
  histories: History | null | (History | null)[] = null,
): string {
  const keys = metricLines(device, metric, histories, [], false);
  if (!keys.includes(key)) keys.push(key);
  const labels = lineLabels(keys.map((k) => ({ key: k, name: lineName(device, metric, k) })));
  return labels[keys.indexOf(key)];
}

export function nfcSource(device: DeviceState, nfc: NfcInfo): Source | null {
  if (device.status !== "ready" && device.status !== "no_module") return null;
  return {
    module: NFC_PRODUCT,
    backend: device.device.backend,
    channel: nfc.channel,
    address: nfc.address,
    part: nfc.part,
  };
}

export function relaySource(device: DeviceState, relay: RelayInfo): Source | null {
  if (device.status !== "ready" && device.status !== "no_module") return null;
  return {
    module: RELAY_PRODUCT,
    backend: device.device.backend,
    channel: relay.channel,
    address: relay.address,
    // The expander's address tells which variant is fitted.
    part: relay.address >= 0x38 ? "TCA9534A" : "TCA9534",
  };
}

/** The GPS Module's receiver: a u-blox SAM-M8Q at 0x42 behind its expander. */
export function gpsSource(device: DeviceState, gps: GpsInfo): Source | null {
  if (device.status !== "ready" && device.status !== "no_module") return null;
  return {
    module: GPS_PRODUCT,
    backend: device.device.backend,
    channel: gps.channel,
    address: 0x42,
    part: "SAM-M8Q",
  };
}
