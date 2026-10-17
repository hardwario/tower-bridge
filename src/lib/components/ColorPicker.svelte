<script lang="ts">
  import { t } from "../i18n/index.svelte";
  import { CHART_COLORS, lineColor } from "../palette";
  import { theme } from "../theme.svelte";
  import type { ChartColor } from "../types";

  /** Swatch button that opens the eight-colour line palette. */
  let { value, onchange }: { value: ChartColor; onchange: (c: ChartColor) => void } = $props();

  let open = $state(false);
  let root: HTMLElement;
  const dark = $derived(theme.effective === "dark");

  $effect(() => {
    if (!open) return;
    const away = (e: PointerEvent) => {
      if (!root.contains(e.target as Node)) open = false;
    };
    const esc = (e: KeyboardEvent) => {
      if (e.key === "Escape") open = false;
    };
    window.addEventListener("pointerdown", away);
    window.addEventListener("keydown", esc);
    return () => {
      window.removeEventListener("pointerdown", away);
      window.removeEventListener("keydown", esc);
    };
  });
</script>

<span class="picker" bind:this={root}>
  <button
    type="button"
    class="trigger"
    title={t().chart.lineColor}
    aria-label={t().chart.lineColor}
    aria-haspopup="true"
    aria-expanded={open}
    onclick={() => (open = !open)}
  >
    <span class="swatch" style:background={lineColor(value, dark)}></span>
  </button>
  {#if open}
    <div class="palette" role="radiogroup" aria-label={t().chart.lineColor}>
      {#each CHART_COLORS as c (c)}
        <button
          type="button"
          role="radio"
          class="choice"
          class:selected={c === value}
          aria-checked={c === value}
          title={t().chart.colors[c]}
          aria-label={t().chart.colors[c]}
          style:--c={lineColor(c, dark)}
          onclick={() => {
            onchange(c);
            open = false;
          }}
        ></button>
      {/each}
    </div>
  {/if}
</span>

<style>
  .picker {
    position: relative;
    display: inline-flex;
  }
  .trigger {
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    padding: 0;
    border: none;
    border-radius: 6px;
    background: none;
    cursor: pointer;
    opacity: 0.6;
    transition: opacity 0.15s;
  }
  :global(.card:hover) .trigger,
  .trigger[aria-expanded="true"],
  .trigger:focus-visible {
    opacity: 1;
  }
  .trigger:hover {
    background: var(--surface-2);
  }
  .swatch {
    width: 12px;
    height: 12px;
    border-radius: 50%;
  }
  .palette {
    position: absolute;
    top: calc(100% + 6px);
    right: 0;
    z-index: 30;
    display: grid;
    grid-template-columns: repeat(4, 22px);
    gap: 8px;
    padding: 10px;
    border-radius: var(--radius-sm);
    border: 1px solid var(--border-strong);
    background: var(--surface);
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.18);
  }
  .choice {
    width: 22px;
    height: 22px;
    padding: 0;
    border: none;
    border-radius: 50%;
    background: var(--c);
    cursor: pointer;
  }
  .choice:hover {
    transform: scale(1.12);
  }
  .choice.selected {
    box-shadow:
      0 0 0 2px var(--surface),
      0 0 0 4px var(--c);
  }
</style>
