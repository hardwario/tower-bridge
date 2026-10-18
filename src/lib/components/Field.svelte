<script lang="ts">
  import type { Snippet } from "svelte";
  let { label, hint, children }: { label: string; hint?: string; children: Snippet } = $props();

  const id = $props.id();
  let control = $state<HTMLElement>();

  // The label names the inputs and selects inside that have no name of
  // their own (toggles carry an aria-label).
  $effect(() => {
    for (const el of control?.querySelectorAll("input, select, textarea") ?? []) {
      if (!el.hasAttribute("aria-label") && !el.hasAttribute("aria-labelledby")) {
        el.setAttribute("aria-labelledby", `${id}-label`);
        if (hint) el.setAttribute("aria-describedby", `${id}-hint`);
      }
    }
  });
</script>

<div class="field">
  <div class="text">
    <div class="label" id="{id}-label">{label}</div>
    {#if hint}<div class="hint muted" id="{id}-hint">{hint}</div>{/if}
  </div>
  <div class="control" bind:this={control}>
    {@render children()}
  </div>
</div>

<style>
  .field {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 11px 14px;
    min-height: 50px;
  }
  .text {
    min-width: 0;
  }
  .label {
    font-weight: 500;
  }
  .hint {
    font-size: 12.5px;
    margin-top: 1px;
  }
  .control {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: none;
  }
</style>
