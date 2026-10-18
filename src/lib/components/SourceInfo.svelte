<script lang="ts">
  import { t } from "../i18n/index.svelte";
  import { sourcePath, sourceText, type Source } from "../source";
  import Icon from "./Icon.svelte";

  /** "i" icon that shows where the values come from on hover or focus (one entry per I²C channel). */
  let { sources }: { sources: Source[] } = $props();

  let info: HTMLElement;
  /** Open to the right when the icon is in the left half of the page: to the
      left the scrolling <main> would clip it. */
  let rightward = $state(false);

  function place() {
    const main = info.closest("main")?.getBoundingClientRect();
    if (main) rightward = info.getBoundingClientRect().left < main.left + main.width / 2;
  }

</script>

<span
  class="info"
  tabindex="0"
  role="button"
  aria-label="{t().common.source}: {sources.map(sourceText).join('; ')}"
  bind:this={info}
  onmouseenter={place}
  onfocus={place}
>
  <Icon name="info" size={14} />
  <span class="popover" class:rightward role="tooltip">
    <span class="title">{t().common.source}</span>
    {#each sources as source (source.channel)}
      <span class="entry">
        <span class="module">{source.module}</span>
        <!-- Wraps only between the steps, never inside one. -->
        <span class="path"
          >{#each sourcePath(source).split(" → ") as step, i (i)}{#if i}{" "}{/if}<span class="step"
              >{i ? "→ " : ""}{step}</span
            >{/each}</span
        >
      </span>
    {/each}
  </span>
</span>

<style>
  .info {
    position: relative;
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    border-radius: 6px;
    color: var(--text-muted);
    cursor: help;
    opacity: 0.6;
    transition: opacity 0.15s;
    outline: none;
  }
  :global(.card:hover) .info,
  .info:focus-visible {
    opacity: 1;
  }
  .info:hover,
  .info:focus-visible {
    color: var(--text);
    background: var(--surface-2);
  }
  .popover {
    display: none;
    position: absolute;
    top: calc(100% + 6px);
    right: 0;
    z-index: 30;
    flex-direction: column;
    gap: 2px;
    width: max-content;
    max-width: 440px;
    padding: 8px 11px;
    border-radius: var(--radius-sm);
    border: 1px solid var(--border-strong);
    background: var(--surface);
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.18);
    color: var(--text);
    font-size: 12.5px;
    line-height: 1.4;
    cursor: default;
  }
  .popover.rightward {
    right: auto;
    left: 0;
  }
  .info:hover .popover,
  .info:focus-visible .popover {
    display: flex;
  }
  .title {
    font-size: 10.5px;
    font-weight: 600;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: var(--text-muted);
  }
  .entry {
    display: flex;
    flex-direction: column;
  }
  .entry + .entry {
    margin-top: 6px;
  }
  .module {
    font-weight: 600;
  }
  .path {
    color: var(--text-2);
  }
  .step {
    white-space: nowrap;
  }
</style>
