<script lang="ts" generics="T extends string">
  let {
    options,
    value,
    label,
    onchange,
    disabled = false,
  }: {
    options: { value: T; label: string }[];
    value: T;
    label: string;
    onchange: (v: T) => void;
    disabled?: boolean;
  } = $props();
</script>

<div class="seg" role="radiogroup" aria-label={label}>
  {#each options as o (o.value)}
    <button
      type="button"
      role="radio"
      aria-checked={o.value === value}
      class:active={o.value === value}
      {disabled}
      onclick={() => onchange(o.value)}>{o.label}</button
    >
  {/each}
</div>

<style>
  .seg {
    display: inline-flex;
    padding: 2px;
    gap: 2px;
    border-radius: var(--radius-sm);
    background: var(--surface-2);
    border: 1px solid var(--border);
  }
  button {
    height: 26px;
    padding: 0 10px;
    border: none;
    border-radius: 6px;
    background: transparent;
    color: var(--text-2);
    font-weight: 500;
    cursor: pointer;
    white-space: nowrap;
  }
  button:hover:not(:disabled) {
    color: var(--text);
  }
  button:disabled {
    cursor: default;
  }
  button:disabled:not(.active) {
    opacity: 0.5;
  }
  button.active {
    background: var(--surface);
    color: var(--text);
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.12);
  }
</style>
