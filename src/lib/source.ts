import { fill, t } from "./i18n/index.svelte";
import type {
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

/** "Climate Module R1 · I²C (FT260) → channel 0 → 0x48 TMP112" */
export function sourceText(s: Source): string {
  const bus = s.backend === "hid" ? "FT260" : "i2c-dev";
  const channel = fill(t().common.channel, { n: s.channel });
  return `${s.module} · I²C (${bus}) → ${channel} → ${hexAddress(s.address)} ${s.part}`;
}

/** Sensor sources while the bridge works, otherwise none. */
export const sourcesOf = (device: DeviceState): SensorSource[] =>
  device.status === "ready" ? (device.sources ?? []) : [];

/** The sensor measuring `metric` on `channel`. */
export function metricSource(device: DeviceState, metric: Metric, channel: number): Source | null {
  if (device.status !== "ready") return null;
  const s = sourcesOf(device).find((x) => x.metric === metric && x.channel === channel);
  if (!s) return null;
  return {
    module: MODULE_NAMES[s.module] ?? s.module,
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
  humidity_tag: { product: "HARDWARIO Humidity Tag", revision: null },
  temperature_tag: { product: "HARDWARIO Temperature Tag", revision: null },
  lux_meter_tag: { product: "HARDWARIO Lux Meter Tag", revision: null },
  barometer_tag: { product: "HARDWARIO Barometer Tag", revision: null },
  co2_module: { product: "TOWER CO2 Module", revision: null },
  voc_lp_tag: { product: "HARDWARIO VOC-LP Tag", revision: null },
};

/**
 * Every module once, in channel order; the Relay Module after the sensors on
 * its channel (it measures nothing, so it is not among the sources).
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
  if (relay) out.push({ channel: relay.channel, product: "TOWER Relay Module", revision: null });
  if (nfc) out.push({ channel: nfc.channel, product: "TOWER NFC Tag", revision: null });
  if (gps) out.push({ channel: gps.channel, product: "TOWER GPS Module", revision: null });
  // Stable: within a channel, the order above stays.
  return out.sort((a, b) => a.channel - b.channel);
}

/** "Climate Module R1 (channel 0), Humidity Tag, Lux Meter Tag (channel 1)" */
export function describeModules(sources: SensorSource[]): string {
  return modulesByChannel(sources)
    .map((e) => `${e.names.join(", ")} (${fill(t().common.channel, { n: e.channel })})`)
    .join(", ");
}

/**
 * Channels with `metric`: measured now, in the latest readings or with data
 * in one of the histories (pass the whole span as well as a zoomed window,
 * so a panel keeps its lines while zooming). Channel 0 when there is nothing
 * at all, so a panel still has its one line, unless `fallback` is off.
 */
export function metricChannels(
  device: DeviceState,
  metric: Metric,
  histories: History | null | (History | null)[],
  latest: Reading[],
  fallback = true,
): number[] {
  const set = new Set<number>();
  for (const s of sourcesOf(device)) if (s.metric === metric) set.add(s.channel);
  for (const r of latest) if (r[metric] !== null) set.add(r.channel);
  for (const h of Array.isArray(histories) ? histories : [histories]) {
    h?.channels.forEach((c, i) => {
      if (h[metric][i]?.avg.some((v) => v !== null)) set.add(c);
    });
  }
  const list = [...set].sort((a, b) => a - b);
  return list.length || !fallback ? list : [0];
}

/**
 * Names for the lines of one panel, in the same order: the module, with the
 * channel ("Humidity Tag · channel 1") when two lines would otherwise read
 * the same.
 */
export function lineLabels(names: { channel: number; name: string }[]): string[] {
  const ambiguous = new Set(names.map((n) => n.name)).size < names.length;
  return names.map((n) => (ambiguous ? `${n.name} · ${fill(t().common.channel, { n: n.channel })}` : n.name));
}

/** Which module measures `metric` on `channel`, e.g. "Humidity Tag"; else "Channel 1". */
export function channelName(device: DeviceState, metric: Metric, channel: number): string {
  const s = sourcesOf(device).find((x) => x.metric === metric && x.channel === channel);
  return s ? (MODULE_NAMES[s.module] ?? s.module) : fill(t().settings.channelN, { n: channel });
}

/** `channelName`, with the channel when another channel's module has the same name. */
export function channelLabel(
  device: DeviceState,
  metric: Metric,
  channel: number,
  histories: History | null | (History | null)[] = null,
): string {
  const channels = metricChannels(device, metric, histories, [], false);
  if (!channels.includes(channel)) channels.push(channel);
  const labels = lineLabels(channels.map((c) => ({ channel: c, name: channelName(device, metric, c) })));
  return labels[channels.indexOf(channel)];
}

export function nfcSource(device: DeviceState, nfc: NfcInfo): Source | null {
  if (device.status !== "ready" && device.status !== "no_module") return null;
  return {
    module: t().nfc.title,
    backend: device.device.backend,
    channel: nfc.channel,
    address: nfc.address,
    part: nfc.part,
  };
}

export function relaySource(device: DeviceState, relay: RelayInfo): Source | null {
  if (device.status !== "ready" && device.status !== "no_module") return null;
  return {
    module: t().relay.title,
    backend: device.device.backend,
    channel: relay.channel,
    address: relay.address,
    // The expander's address tells which variant is fitted.
    part: relay.address >= 0x38 ? "TCA9534A" : "TCA9534",
  };
}
