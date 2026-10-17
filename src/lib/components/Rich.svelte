<script lang="ts">
  /** Text with `{name}` placeholders rendered as inline code. */
  let { text, codes }: { text: string; codes: Record<string, string> } = $props();

  const parts = $derived(
    text.split(/(\{\w+\})/g).map((part) => {
      const key = /^\{(\w+)\}$/.exec(part)?.[1];
      return key && key in codes ? { code: codes[key] } : { text: part };
    }),
  );
</script>

{#each parts as part, i (i)}{#if "code" in part}<code>{part.code}</code>{:else}{part.text}{/if}{/each}
