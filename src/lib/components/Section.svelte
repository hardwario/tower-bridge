<script lang="ts">
  import type { Snippet } from "svelte";
  let { title, description, children }: { title: string; description?: string; children: Snippet } = $props();
</script>

<section>
  <header>
    <h2>{title}</h2>
    {#if description}<p class="muted">{description}</p>{/if}
  </header>
  <div class="card body">
    {@render children()}
  </div>
</section>

<style>
  section {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  header {
    padding: 0 4px;
  }
  h2 {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-2);
  }
  p {
    margin: 2px 0 0;
    font-size: 12.5px;
  }
  .body {
    display: flex;
    flex-direction: column;
  }
  .body > :global(* + *) {
    border-top: 1px solid var(--border);
  }
</style>
