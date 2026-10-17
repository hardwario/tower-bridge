<script lang="ts">
  import type { Snippet } from "svelte";
  import Icon from "./Icon.svelte";
  let {
    kind = "info",
    title,
    children,
    actions,
  }: { kind?: "info" | "warning" | "error"; title: string; children?: Snippet; actions?: Snippet } = $props();
</script>

<div class="banner {kind}" role={kind === "info" ? "status" : "alert"}>
  <span class="icon"><Icon name={kind === "info" ? "info" : "warning"} size={18} /></span>
  <div class="content">
    <div class="title">{title}</div>
    {#if children}<div class="body">{@render children()}</div>{/if}
  </div>
  {#if actions}<div class="actions">{@render actions()}</div>{/if}
</div>

<style>
  .banner {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 12px 14px;
    border-radius: var(--radius);
    border: 1px solid var(--border);
    background: var(--surface);
  }
  .banner.warning {
    background: var(--warning-weak);
    border-color: transparent;
  }
  .banner.error {
    background: var(--critical-weak);
    border-color: transparent;
  }
  .icon {
    display: flex;
    margin-top: 1px;
    color: var(--text-2);
  }
  .warning .icon {
    color: var(--warning);
  }
  .error .icon {
    color: var(--critical);
  }
  .content {
    flex: 1;
    min-width: 0;
  }
  .title {
    font-weight: 600;
  }
  .body {
    margin-top: 2px;
    color: var(--text-2);
    font-size: 13px;
    -webkit-user-select: text;
    user-select: text;
  }
  .body :global(code) {
    display: inline-block;
    padding: 1px 5px;
    border-radius: 4px;
    background: var(--surface-2);
  }
  .actions {
    display: flex;
    gap: 8px;
    flex: none;
  }
</style>
