<script lang="ts">
  import { t } from "../i18n/index.svelte";
  import Icon from "./Icon.svelte";

  /** Grip for reordering overview panels; dragging starts on pointer down. */
  let { onstart }: { onstart: (e: PointerEvent) => void } = $props();
</script>

<button
  type="button"
  class="grip"
  title={t().overview.dragToReorder}
  aria-label={t().overview.dragToReorder}
  onpointerdown={onstart}><Icon name="grip" size={15} /></button
>

<style>
  .grip {
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    padding: 0;
    border: none;
    border-radius: 6px;
    background: none;
    color: var(--text-muted);
    cursor: grab;
    opacity: 0.5;
    transition: opacity 0.15s;
    touch-action: none;
    flex-shrink: 0;
  }
  :global(.card:hover) .grip,
  :global(.dragging) .grip,
  .grip:focus-visible {
    opacity: 1;
  }
  .grip:hover {
    color: var(--text);
    background: var(--surface-2);
  }
  :global(.dragging) .grip {
    cursor: grabbing;
  }
</style>
