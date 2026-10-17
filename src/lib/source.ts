import { fill, t } from "./i18n/index.svelte";
import type { DeviceInfo, DeviceState, Metric, RelayInfo } from "./types";

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

export function metricSource(device: DeviceState, metric: Metric): Source | null {
  if (device.status !== "ready") return null;
  const s = device.sources?.find((x) => x.metric === metric);
  if (!s) return null;
  return {
    module: fill(t().overview.climateModule, { revision: device.revision }),
    backend: device.device.backend,
    channel: device.channel,
    address: s.address,
    part: s.sensor,
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
