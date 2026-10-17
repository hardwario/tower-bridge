<script lang="ts">
  import type { Snippet } from "svelte";
  let { label, hint, children }: { label: string; hint?: string; children: Snippet } = $props();
</script>

<div class="field">
  <div class="text">
    <div class="label">{label}</div>
    {#if hint}<div class="hint muted">{hint}</div>{/if}
  </div>
  <div class="control">
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
