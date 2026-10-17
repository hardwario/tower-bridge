<script lang="ts">
  let {
    checked,
    disabled = false,
    label,
    onchange,
  }: { checked: boolean; disabled?: boolean; label: string; onchange: (v: boolean) => void } = $props();
</script>

<button
  type="button"
  role="switch"
  class="toggle"
  aria-checked={checked}
  aria-label={label}
  {disabled}
  onclick={() => onchange(!checked)}
>
  <span class="knob"></span>
</button>

<style>
  .toggle {
    position: relative;
    width: 36px;
    height: 20px;
    padding: 0;
    border-radius: 10px;
    border: none;
    background: var(--border-strong);
    cursor: pointer;
    flex: none;
    transition: background 0.15s;
  }
  .toggle[aria-checked="true"] {
    background: var(--accent);
  }
  .toggle:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .knob {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: #fff;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.25);
    transition: transform 0.15s;
  }
  .toggle[aria-checked="true"] .knob {
    transform: translateX(16px);
  }
</style>
