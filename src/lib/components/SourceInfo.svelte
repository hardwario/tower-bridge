<script lang="ts">
  import { fill, t } from "../i18n/index.svelte";
  import { hexAddress, sourceText, type Source } from "../source";
  import Icon from "./Icon.svelte";

  /** "i" icon that shows where the values come from on hover or focus (one entry per I²C channel). */
  let { sources }: { sources: Source[] } = $props();

  const bus = (s: Source) => (s.backend === "hid" ? "FT260" : "i2c-dev");
</script>

<span class="info" tabindex="0" role="button" aria-label="{t().common.source}: {sources.map(sourceText).join('; ')}">
  <Icon name="info" size={14} />
  <span class="popover" role="tooltip">
    <span class="title">{t().common.source}</span>
    {#each sources as source (source.channel)}
      <span class="entry">
        <span class="module">{source.module}</span>
        <span class="path">
          I²C ({bus(source)}) → {fill(t().common.channel, { n: source.channel })} →
          <span class="mono">{hexAddress(source.address)}</span>
          {source.part}
        </span>
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
    max-width: 320px;
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
    white-space: nowrap;
  }
  .mono {
    font-size: 12px;
  }
</style>
