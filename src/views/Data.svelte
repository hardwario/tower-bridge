<script lang="ts">
  /**
   * The stored readings as a table, with CSV and XLSX export of a time range.
   * Charts are on the Dashboard only.
   */
  import { onDestroy, onMount } from "svelte";
  import { api, errorMessage, on } from "../lib/api";
  import { hasValueAt, seriesOf } from "../lib/chart-group.svelte";
  import Icon from "../lib/components/Icon.svelte";
  import SegmentedControl from "../lib/components/SegmentedControl.svelte";
  import { excelDateTimeFormat, fmtDateTime, fmtDuration, fmtValue } from "../lib/format";
  import { fill, plural, t } from "../lib/i18n/index.svelte";
  import { live } from "../lib/live.svelte";
  import { SPANS, fromLocalInput, toLocalInput, type Span } from "../lib/time";
  import { channelLabel, metricChannels } from "../lib/source";
  import { METRICS, type ExportResult, type History } from "../lib/types";
  import { unitLabel } from "../lib/units";

  /** The Dashboard's spans, or dates of one's own. */
  let range = $state<Span | "custom">("24h");
  let customFrom = $state(toLocalInput(Date.now() - 86_400_000));
  let customTo = $state(toLocalInput(Date.now()));
  // Replaced whole, never changed in place: no deep reactivity needed.
  let history = $state.raw<History | null>(null);
  /** Only the newest load may land (an older, slower one must not overwrite it). */
  let loads = 0;
  let loading = $state(false);
  let error = $state<string | null>(null);
  let exporting = $state<{ done: number; total: number } | null>(null);
  let exported = $state<ExportResult | null>(null);
  let exportError = $state<string | null>(null);
  let lastFetch = 0;

  const unit = $derived(live.settings?.temperature_unit ?? "celsius");

  function bounds(): [number, number] {
    if (range === "custom") {
      const a = fromLocalInput(customFrom);
      const b = fromLocalInput(customTo);
      return a < b ? [a, b] : [b, a];
    }
    const now = Date.now();
    const span = SPANS.find((s) => s.key === range) ?? SPANS[2];
    return [now - span.ms, now + 1];
  }

  async function load() {
    const [from, to] = bounds();
    if (!Number.isFinite(from) || !Number.isFinite(to)) return;
    loading = true;
    lastFetch = Date.now();
    const mine = ++loads;
    try {
      const h = await api.history(from, to, 900);
      if (mine !== loads) return;
      history = h;
      error = null;
    } catch (e) {
      error = errorMessage(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    void [range, customFrom, customTo];
    load();
  });

  // Keep live ranges current.
  $effect(() => {
    void live.tick;
    const span = SPANS.find((s) => s.key === range);
    if (span && Date.now() - lastFetch >= span.refreshMs) load();
  });

  let stop: (() => void) | null = null;
  onMount(async () => {
    stop = await on("export-progress", (p) => (exporting = p));
  });
  onDestroy(() => stop?.());

  async function exportFile(format: "csv" | "xlsx") {
    const [from, to] = bounds();
    exported = null;
    exportError = null;
    exporting = { done: 0, total: 0 };
    try {
      exported = format === "csv" ? await api.exportCsv(from, to) : await api.exportXlsx(from, to, excelDateTimeFormat());
    } catch (e) {
      exportError = errorMessage(e);
    } finally {
      exporting = null;
    }
  }

  /** Table columns: each metric on each channel that measured it. */
  const columns = $derived(
    history
      ? METRICS.flatMap((m) =>
          metricChannels(live.device, m, history, [], false).map((channel) => ({ key: `${m}-${channel}`, m, channel })),
        )
      : [],
  );

  /** The column the rows are sorted by ("time" or a column key); newest first to start with. */
  let sort = $state<{ key: string; desc: boolean }>({ key: "time", desc: true });

  /** A click on the sorted column turns the order round; another column starts with the highest value. */
  function sortBy(key: string) {
    sort = sort.key === key ? { key, desc: !sort.desc } : { key, desc: true };
  }

  /** Rows with any value, in the chosen order; rows without a value in the sorted column go last. */
  const rows = $derived.by(() => {
    if (!history) return [];
    const h = history;
    const out = h.t.map((t, i) => ({ t: t * 1000, i })).filter(({ i }) => hasValueAt(h, i));
    const column = columns.find((c) => c.key === sort.key);
    const value = (i: number): number | null =>
      column ? (seriesOf(h, column.m, column.channel)?.avg[i] ?? null) : h.t[i];
    const dir = sort.desc ? -1 : 1;
    return out.sort((a, b) => {
      const [x, y] = [value(a.i), value(b.i)];
      if (x === null || y === null) return x === y ? b.t - a.t : x === null ? 1 : -1;
      return x === y ? b.t - a.t : (x - y) * dir;
    });
  });

  const ariaSort = (key: string) => (sort.key === key ? (sort.desc ? "descending" : "ascending") : "none");
  const severalChannels = $derived(new Set(columns.map((c) => c.channel)).size > 1);

  const empty = $derived(!!history && rows.length === 0);
</script>

<div class="view data">
  <header class="page-head">
    <h1>{t().data.title}</h1>
  </header>

  <div class="filters">
    <SegmentedControl
      label={t().data.range}
      value={range}
      options={[
        ...SPANS.map((s) => ({ value: s.key as Span | "custom", label: t().chart.ranges[s.key] })),
        { value: "custom" as const, label: t().data.custom },
      ]}
      onchange={(v) => (range = v)}
    />
    <span class="spacer"></span>
    <div class="exports">
      <button class="btn" disabled={!!exporting || empty} onclick={() => exportFile("csv")}>
        <Icon name="download" size={14} />
        {t().data.exportCsv}
      </button>
      <button class="btn" disabled={!!exporting || empty} onclick={() => exportFile("xlsx")}>
        <Icon name="download" size={14} />
        {t().data.exportXlsx}
      </button>
    </div>
  </div>

  <!-- Its own row, so choosing it moves nothing in the row above. -->
  {#if range === "custom"}
    <div class="custom-range">
      <input class="input" type="datetime-local" bind:value={customFrom} aria-label={t().data.from} />
      <span class="muted">{t().data.to}</span>
      <input class="input" type="datetime-local" bind:value={customTo} aria-label={t().data.to} />
    </div>
  {/if}

  {#if exporting}
    <div class="note muted">
      {t().data.exporting}
      {exporting.total ? `${Math.round((exporting.done / exporting.total) * 100)} %` : ""}
    </div>
  {:else if exported}
    <div class="note">
      <span>{plural(t().data.exported, exported.rows, { path: exported.path })}</span>
      <button class="btn ghost" onclick={() => exported && api.revealFile(exported.path)}>
        <Icon name="folder" size={14} />
        {t().data.reveal}
      </button>
    </div>
  {:else if exportError}
    <div class="note error">{fill(t().data.exportFailed, { error: exportError })}</div>
  {/if}

  {#if error}
    <div class="note error">{fill(t().data.loadFailed, { error })}</div>
  {/if}

  {#if empty}
    <div class="card empty">
      <Icon name="table" size={28} stroke={1.5} />
      <div>{t().data.empty}</div>
    </div>
  {:else if history}
    <div class="card table-wrap" class:loading>
      <table class="tabular">
        <thead>
          <tr>
            {#snippet arrow(key: string)}
              {#if sort.key === key}<span class="arrow" class:up={!sort.desc}><Icon name="chevron" size={12} /></span>{/if}
            {/snippet}
            <th aria-sort={ariaSort("time")}>
              <button class="sort" title={t().data.sortHint} onclick={() => sortBy("time")}>
                <span class="head"><span>{t().data.time}{@render arrow("time")}</span></span>
              </button>
            </th>
            {#each columns as c (c.key)}
              <th aria-sort={ariaSort(c.key)}>
                <button class="sort" title={t().data.sortHint} onclick={() => sortBy(c.key)}>
                  <span class="head">
                    <span>{t().metrics[c.m]} ({unitLabel(c.m, unit)}){@render arrow(c.key)}</span>
                    {#if severalChannels}<span class="sub">{channelLabel(live.device, c.m, c.channel, history)}</span>{/if}
                  </span>
                </button>
              </th>
            {/each}
          </tr>
        </thead>
        <tbody>
          {#each rows as r (r.t)}
            <tr>
              <td>{fmtDateTime(r.t, { year: true })}</td>
              {#each columns as c (c.key)}
                <td>{fmtValue(c.m, seriesOf(history, c.m, c.channel)?.avg[r.i] ?? null, unit)}</td>
              {/each}
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
    <div class="muted small">{fill(t().data.averages, { bucket: fmtDuration(history.bucket_ms) })}</div>
  {/if}
</div>

<style>
  .filters {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
  }
  .custom-range {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
  }
  .spacer {
    flex: 1;
  }
  /* Both export buttons always wrap together. */
  .exports {
    display: inline-flex;
    gap: 8px;
    flex-shrink: 0;
  }
  .note {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
    -webkit-user-select: text;
    user-select: text;
  }
  .note.error {
    color: var(--critical);
  }
  .loading {
    opacity: 0.6;
  }
  th .sub {
    font-size: 11.5px;
    font-weight: 400;
    color: var(--text-muted);
  }
  /* The whole head is the sort button, looking like the head. */
  .sort {
    margin: -4px -6px;
    padding: 4px 6px;
    border: none;
    border-radius: 6px;
    background: none;
    font: inherit;
    color: inherit;
    text-align: inherit;
    cursor: pointer;
  }
  .sort:hover {
    background: var(--surface-2);
    color: var(--text);
  }
  .head {
    display: inline-flex;
    flex-direction: column;
    align-items: flex-end;
  }
  th:first-child .head {
    align-items: flex-start;
  }
  .arrow {
    display: inline-flex;
    margin-left: 4px;
    vertical-align: -1px;
  }
  .arrow.up {
    transform: rotate(180deg);
  }
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    padding: 48px;
    color: var(--text-muted);
  }
  /* The page fills the window and the table takes the height left below the
     controls, scrolling inside; a small window scrolls the page instead. */
  .data {
    height: 100%;
  }
  .table-wrap {
    flex: 1 1 auto;
    min-height: 220px;
    overflow: auto;
    transition: opacity 0.2s;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
  }
  th,
  td {
    padding: 7px 14px;
    text-align: right;
    white-space: nowrap;
  }
  th:first-child,
  td:first-child {
    text-align: left;
  }
  th {
    position: sticky;
    top: 0;
    background: var(--surface);
    font-weight: 600;
    color: var(--text-2);
    border-bottom: 1px solid var(--border);
  }
  tbody tr + tr td {
    border-top: 1px solid var(--border);
  }
  /* The time stays in view when the columns scroll sideways. */
  th:first-child,
  td:first-child {
    position: sticky;
    left: 0;
    background: var(--surface);
    box-shadow: inset -1px 0 0 var(--border);
  }
  th:first-child {
    z-index: 1;
  }
  .small {
    font-size: 12.5px;
  }
</style>
