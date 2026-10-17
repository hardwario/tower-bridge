<script lang="ts">
  import type { ChartGroup } from "../chart-group.svelte";
  import { t } from "../i18n/index.svelte";
  import Icon from "./Icon.svelte";

  /** Back to the live time span; the charts of a view share one window. */
  let { group }: { group: ChartGroup } = $props();
</script>

<button
  type="button"
  class="reset"
  title={t().chart.resetZoom}
  aria-label={t().chart.resetZoom}
  disabled={!group.zoom}
  onclick={() => group.reset()}><Icon name="reset" size={14} /></button
>

<style>
  .reset {
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    padding: 0;
    border: none;
    border-radius: 6px;
    background: none;
    color: var(--text-muted);
    cursor: pointer;
    opacity: 0.5;
    transition: opacity 0.15s;
    flex-shrink: 0;
  }
  :global(.card:hover) .reset:not(:disabled),
  .reset:focus-visible {
    opacity: 1;
  }
  .reset:hover:not(:disabled) {
    color: var(--text);
    background: var(--surface-2);
  }
  /* Nothing to reset: still there, so the header does not shift. */
  .reset:disabled {
    opacity: 0.2;
    cursor: default;
  }
</style>
