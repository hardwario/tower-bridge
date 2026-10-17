<script lang="ts">
  import { t } from "../i18n/index.svelte";
  import Icon from "./Icon.svelte";
  let { label, text }: { label?: string; text: string } = $props();
  let copied = $state(false);

  async function copy() {
    try {
      await navigator.clipboard.writeText(text);
      copied = true;
      setTimeout(() => (copied = false), 2000);
    } catch (e) {
      console.error(e);
    }
  }
</script>

<div class="block">
  {#if label}<div class="label muted">{label}</div>{/if}
  <div class="row">
    <pre class="mono">{text}</pre>
    <button class="btn ghost" onclick={copy} title={t().common.copy} aria-label="{t().common.copy} {label ?? ''}">
      <Icon name={copied ? "check" : "copy"} size={14} />
    </button>
  </div>
</div>

<style>
  .block {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .label {
    font-size: 12.5px;
  }
  .row {
    display: flex;
    align-items: flex-start;
    gap: 6px;
  }
  pre {
    flex: 1;
    min-width: 0;
    margin: 0;
    padding: 7px 10px;
    border-radius: var(--radius-sm);
    background: var(--surface-2);
    overflow-x: auto;
    white-space: pre;
    -webkit-user-select: text;
    user-select: text;
  }
</style>
