<script lang="ts">
  import { fill, t } from "../i18n/index.svelte";
  import type { DeviceState } from "../types";
  let { device }: { device: DeviceState } = $props();

  const info = $derived.by(() => {
    const s = t().status;
    switch (device.status) {
      case "ready":
        return { tone: "good", text: fill(s.module, { revision: device.revision }) };
      case "no_module":
        return { tone: "warn", text: s.noModule };
      case "searching":
        return { tone: "idle", text: s.searching };
      case "error":
        return device.kind === "not_found" ? { tone: "idle", text: s.notConnected } : { tone: "warn", text: s.attention };
    }
  });
</script>

<span class="pill {info.tone}">
  <span class="dot"></span>
  {info.text}
</span>

<style>
  .pill {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    font-size: 12.5px;
    font-weight: 500;
    color: var(--text-2);
    white-space: nowrap;
  }
  .dot {
    flex-shrink: 0;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--text-muted);
  }
  .good .dot {
    background: var(--good);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--good) 20%, transparent);
  }
  .warn .dot {
    background: var(--warning);
  }
</style>
