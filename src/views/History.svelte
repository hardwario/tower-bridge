<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { api, errorMessage, on } from "../lib/api";
  import { ChartGroup, dataExtent, panBounds, windowStats, zoomDetail } from "../lib/chart-group.svelte";
  import ChartStats from "../lib/components/ChartStats.svelte";
  import ColorPicker from "../lib/components/ColorPicker.svelte";
  import ResetView from "../lib/components/ResetView.svelte";
  import SourceInfo from "../lib/components/SourceInfo.svelte";
  import TimeChart from "../lib/components/TimeChart.svelte";
  import Icon from "../lib/components/Icon.svelte";
  import SegmentedControl from "../lib/components/SegmentedControl.svelte";
  import { excelDateTimeFormat, fmtDateTime, fmtDuration, fmtValue } from "../lib/format";
  import { fill, plural, t } from "../lib/i18n/index.svelte";
  import { live } from "../lib/live.svelte";
  import { RANGES, fromLocalInput, toLocalInput, type RangeKey } from "../lib/time";
  import { chartStyle, lineColor } from "../lib/palette";
  import { metricSource } from "../lib/source";
  import { theme } from "../lib/theme.svelte";
  import { METRICS, type ChartStyle, type ExportResult, type History } from "../lib/types";
  import { unitLabel } from "../lib/units";

  let range = $state<RangeKey>("24h");
  let view = $state<"chart" | "table">("chart");
  let customFrom = $state(toLocalInput(Date.now() - 86_400_000));
  let customTo = $state(toLocalInput(Date.now()));
  let history = $state<History | null>(null);
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
    const r = RANGES.find((x) => x.key === range)!;
    return [now - r.ms, now + 1];
  }

  async function load() {
    const [from, to] = bounds();
    if (!Number.isFinite(from) || !Number.isFinite(to)) return;
    loading = true;
    lastFetch = Date.now();
    try {
      const [h, range] = await Promise.all([api.history(from, to, 900), api.dataRange()]);
      history = h;
      group.home = [h.from / 1000, h.to / 1000];
      group.bounds = panBounds(range, h);
      error = null;
    } catch (e) {
      error = errorMessage(e);
    } finally {
      loading = false;
    }
  }

  const group = new ChartGroup("history");
  const detail = zoomDetail(group, 900);
  const shown = $derived(detail.view(history));
  // What panning must keep in view: the loaded window's data, or the span's.
  $effect(() => {
    group.loaded = dataExtent(group.zoom && detail.current ? detail.current : history);
  });

  $effect(() => {
    void [range, customFrom, customTo];
    group.reset();
    load();
  });

  // Keep live ranges current, but never redraw under the pointer.
  $effect(() => {
    void live.tick;
    if (range === "custom" || group.hovering) return;
    const minGap = range === "1h" ? 5_000 : 60_000;
    if (Date.now() - lastFetch >= minGap) load();
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

  const rows = $derived.by(() => {
    if (!history) return [];
    const h = history;
    const out = h.t.map((t, i) => ({ t: t * 1000, i })).filter(({ i }) => METRICS.some((m) => h[m].avg[i] !== null));
    return out.reverse();
  });

  const empty = $derived(!!history && rows.length === 0);
</script>

<div class="view">
  <header class="page-head">
    <h1>{t().history.title}</h1>
  </header>

  <div class="filters">
    <SegmentedControl
      label={t().history.range}
      value={range}
      options={[
        ...RANGES.map((r) => ({ value: r.key, label: t().history.ranges[r.key as Exclude<RangeKey, "custom">] })),
        { value: "custom" as RangeKey, label: t().history.custom },
      ]}
      onchange={(v) => (range = v)}
    />
    {#if range === "custom"}
      <input class="input" type="datetime-local" bind:value={customFrom} aria-label={t().history.from} />
      <span class="muted">{t().history.to}</span>
      <input class="input" type="datetime-local" bind:value={customTo} aria-label={t().history.to} />
    {/if}
    {#if group.zoom && view === "chart"}
      <button class="btn" onclick={() => group.reset()}><Icon name="refresh" size={14} /> {t().chart.resetZoom}</button>
    {/if}
    <span class="spacer"></span>
    <SegmentedControl
      label={t().history.view}
      value={view}
      options={[
        { value: "chart", label: t().history.chart },
        { value: "table", label: t().history.table },
      ]}
      onchange={(v) => (view = v)}
    />
    <div class="exports">
      <button class="btn" disabled={!!exporting || empty} onclick={() => exportFile("csv")}>
        <Icon name="download" size={14} /> {t().history.exportCsv}
      </button>
      <button class="btn" disabled={!!exporting || empty} onclick={() => exportFile("xlsx")}>
        <Icon name="download" size={14} /> {t().history.exportXlsx}
      </button>
    </div>
  </div>

  {#if exporting}
    <div class="note muted">
      {t().history.exporting}
      {exporting.total ? `${Math.round((exporting.done / exporting.total) * 100)} %` : ""}
    </div>
  {:else if exported}
    <div class="note">
      <span>{plural(t().history.exported, exported.rows, { path: exported.path })}</span>
      <button class="btn ghost" onclick={() => exported && api.revealFile(exported.path)}>
        <Icon name="folder" size={14} />
        {t().history.reveal}
      </button>
    </div>
  {:else if exportError}
    <div class="note error">{fill(t().history.exportFailed, { error: exportError })}</div>
  {/if}

  {#if error}
    <div class="note error">{fill(t().history.loadFailed, { error })}</div>
  {/if}

  {#if empty}
    <div class="card empty">
      <Icon name="chart" size={28} stroke={1.5} />
      <div>{t().history.empty}</div>
    </div>
  {:else if shown && view === "chart"}
    <div class="charts" class:loading>
      {#each METRICS as m (m)}
        {@const st = windowStats(shown, m, group.zoom)}
        {@const style = chartStyle(live.settings?.charts, m)}
        {@const color = lineColor(style.color, theme.effective === "dark")}
        {@const source = metricSource(live.device, m)}
        {@const setStyle = (next: ChartStyle) => live.save((x) => (x.charts = { ...x.charts, [m]: next }))}
        <section class="card chart-card">
          <header>
            <h2>
              {t().metrics[m]} <span class="muted">{unitLabel(m, unit)}</span>
              <ColorPicker value={style.color} onchange={(c) => setStyle({ ...style, color: c })} />
              {#if source}<SourceInfo {source} />{/if}
              <ResetView {group} />
            </h2>
            <ChartStats metric={m} {unit} stats={st} {style} {color} onchange={setStyle} />
          </header>
          <TimeChart
            metric={m}
            history={shown}
            {unit}
            {group}
            rule={live.settings?.alerts[m]}
            {color}
            show={{ min: style.show_min, avg: style.show_avg, max: style.show_max }}
            height={190}
          />
        </section>
      {/each}
    </div>
  {:else if history}
    <div class="card table-wrap" class:loading>
      <table class="tabular">
        <thead>
          <tr>
            <th>{t().history.time}</th>
            {#each METRICS as m (m)}<th>{t().metrics[m]} ({unitLabel(m, unit)})</th>{/each}
          </tr>
        </thead>
        <tbody>
          {#each rows as r (r.t)}
            <tr>
              <td>{fmtDateTime(r.t, { year: true })}</td>
              {#each METRICS as m (m)}<td>{fmtValue(m, history[m].avg[r.i], unit)}</td>{/each}
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
    <div class="muted small">{fill(t().history.averages, { bucket: fmtDuration(history.bucket_ms) })}</div>
  {/if}
</div>

<style>
  .filters {
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
  .charts {
    display: flex;
    flex-direction: column;
    gap: 12px;
    transition: opacity 0.2s;
  }
  .loading {
    opacity: 0.6;
  }
  .chart-card {
    padding: 12px 14px 8px;
  }
  .chart-card header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    margin-bottom: 4px;
  }
  h2 {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 14px;
  }
  h2 .muted {
    font-weight: 400;
  }
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    padding: 48px;
    color: var(--text-muted);
  }
  .table-wrap {
    max-height: calc(100vh - 230px);
    overflow: auto;
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
  .small {
    font-size: 12.5px;
  }
</style>
