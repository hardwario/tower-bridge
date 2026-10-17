<script lang="ts">
  import { api, errorCode, errorMessage } from "../api";
  import { fill, t } from "../i18n/index.svelte";
  import { live } from "../live.svelte";
  import { nfcSource } from "../source";
  import type { NfcInfo } from "../types";
  import DragHandle from "./DragHandle.svelte";
  import Icon from "./Icon.svelte";
  import SourceInfo from "./SourceInfo.svelte";

  let { nfc, ondragstart }: { nfc: NfcInfo; ondragstart?: (e: PointerEvent) => void } = $props();

  /** In UTF-8 bytes, as the tag stores it (nfc::MAX_URL). */
  const MAX_URL = 240;

  const m = $derived(t().nfc);
  const source = $derived(nfcSource(live.device, nfc));
  let draft = $state("");
  /** The draft follows the tag until the user edits it. */
  let edited = $state(false);
  let busy = $state(false);
  let error = $state<string | null>(null);

  $effect(() => {
    const url = nfc.url ?? "";
    if (!edited) draft = url;
  });

  /** A bare host name gets https://; anything with a scheme stays as it is. */
  function normalized(text: string): string {
    const s = text.trim();
    return s === "" || /^[a-z][a-z0-9+.-]*:/i.test(s) ? s : `https://${s}`;
  }

  const target = $derived(normalized(draft));

  async function write(e: SubmitEvent) {
    e.preventDefault();
    if (busy || !target) return;
    if (new TextEncoder().encode(target).length > MAX_URL) {
      error = fill(m.tooLong, { max: MAX_URL });
      return;
    }
    busy = true;
    error = null;
    try {
      await api.writeNfcUrl(target);
      edited = false;
    } catch (err) {
      error =
        errorCode(err) === "not_connected"
          ? t().overview.notConnectedError
          : fill(m.failed, { error: errorMessage(err) });
    } finally {
      busy = false;
    }
  }
</script>

<section class="card nfc">
  <div class="head">
    <h2><Icon name="nfc" size={16} /> {m.title}</h2>
    <span class="spacer"></span>
    {#if source}<SourceInfo sources={[source]} />{/if}
    {#if ondragstart}<DragHandle onstart={ondragstart} />{/if}
  </div>

  <p class="current">
    <span class="muted">{m.current}</span>
    {#if nfc.url}
      <span class="url mono">{nfc.url}</span>
    {:else}
      <span class="muted">{m.none}</span>
    {/if}
  </p>

  {#if nfc.formatted}
    <form class="write" onsubmit={write}>
      <input
        class="input"
        type="text"
        inputmode="url"
        spellcheck="false"
        autocomplete="off"
        placeholder="https://…"
        aria-label={m.url}
        bind:value={draft}
        oninput={() => {
          edited = true;
          error = null;
        }}
        disabled={busy}
      />
      <button class="btn" type="submit" disabled={busy || !target || target === (nfc.url ?? "")} aria-busy={busy}>
        <Icon name={busy ? "loader" : "nfc"} size={14} />
        {busy ? m.writing : m.write}
      </button>
    </form>
    <p class="detail" class:error={!!error}>
      {#if error}<Icon name="warning" size={14} /> {error}{:else}{m.hint}{/if}
    </p>
  {:else}
    <p class="detail error"><Icon name="warning" size={14} /> {m.notFormatted}</p>
  {/if}
</section>

<style>
  .nfc {
    padding: 16px 18px 14px;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  h2 {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    font-size: 14px;
  }
  .spacer {
    flex: 1;
  }
  .current {
    display: flex;
    align-items: baseline;
    flex-wrap: wrap;
    gap: 4px 10px;
    margin: 12px 0 0;
    font-size: 13px;
  }
  .url {
    overflow-wrap: anywhere;
    color: var(--text);
    -webkit-user-select: text;
    user-select: text;
  }
  .write {
    display: flex;
    gap: 8px;
    margin-top: 10px;
  }
  .write .input {
    flex: 1;
    min-width: 0;
    max-width: 520px;
  }
  .write .btn {
    flex-shrink: 0;
  }
  .detail {
    display: flex;
    align-items: center;
    gap: 6px;
    min-height: 18px;
    margin: 10px 0 0;
    font-size: 12.5px;
    color: var(--text-muted);
  }
  .detail.error {
    color: var(--warning);
  }
</style>
