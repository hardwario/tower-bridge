<script lang="ts">
  import { onMount } from "svelte";
  import { flip } from "svelte/animate";
  import { api } from "../lib/api";
  import { ChartGroup, dataExtent, panBounds, windowStats, zoomDetail, zoomHint } from "../lib/chart-group.svelte";
  import { deviceActions } from "../lib/device-actions.svelte";
  import Icon from "../lib/components/Icon.svelte";
  import MetricCard from "../lib/components/MetricCard.svelte";
  import RelayCard from "../lib/components/RelayCard.svelte";
  import SegmentedControl from "../lib/components/SegmentedControl.svelte";
  import { fmtDateTime, fmtDuration, fmtNumber } from "../lib/format";
  import { fill, t } from "../lib/i18n/index.svelte";
  import { live } from "../lib/live.svelte";
  import { layout } from "../lib/overview-layout.svelte";
  import { chartStyle } from "../lib/palette";
  import { metricSource } from "../lib/source";
  import type { History, Metric } from "../lib/types";
  import { altitude } from "../lib/units";
  import DragHandle from "../lib/components/DragHandle.svelte";
  import DeviceBanners from "./DeviceBanners.svelte";

  type Span = "1h" | "6h" | "24h" | "7d" | "30d" | "90d" | "365d";
  const DAY = 86_400_000;
  /** Longer spans refresh less often: a new reading barely moves them. */
  const SPANS: { key: Span; ms: number; refreshMs: number }[] = [
    { key: "1h", ms: 3_600_000, refreshMs: 5_000 },
    { key: "6h", ms: 6 * 3_600_000, refreshMs: 30_000 },
    { key: "24h", ms: DAY, refreshMs: 60_000 },
    { key: "7d", ms: 7 * DAY, refreshMs: 300_000 },
    { key: "30d", ms: 30 * DAY, refreshMs: 900_000 },
    { key: "90d", ms: 90 * DAY, refreshMs: 1_800_000 },
    { key: "365d", ms: 365 * DAY, refreshMs: 3_600_000 },
  ];
  const SPAN_KEY = "tower-bridge.overview.span";

  function savedSpan(): Span {
    try {
      const v = localStorage.getItem(SPAN_KEY);
      if (SPANS.some((s) => s.key === v)) return v as Span;
    } catch {
      /* storage unavailable */
    }
    return "24h";
  }

  let span = $state<Span>(savedSpan());
  let history = $state<History | null>(null);
  let lastFetch = 0;
  const group = new ChartGroup("overview");
  const detail = zoomDetail(group, 240);
  const shown = $derived(detail.view(history));
  // What panning must keep in view: the loaded window's data, or the span's.
  $effect(() => {
    group.loaded = dataExtent(group.zoom && detail.current ? detail.current : history);
  });

  async function load() {
    const conf = SPANS.find((s) => s.key === span) ?? SPANS[2];
    const t = Date.now();
    lastFetch = t;
    try {
      const [h, range] = await Promise.all([api.history(t - conf.ms, t + 1, 240), api.dataRange()]);
      history = h;
      group.home = [h.from / 1000, h.to / 1000];
      group.bounds = panBounds(range, h);
    } catch (e) {
      console.error(e);
    }
  }

  function setSpan(v: Span) {
    span = v;
    group.reset();
    try {
      localStorage.setItem(SPAN_KEY, v);
    } catch {
      /* storage unavailable */
    }
    load();
  }

  // Follow new readings, but never redraw under the pointer.
  $effect(() => {
    void live.tick;
    const conf = SPANS.find((s) => s.key === span) ?? SPANS[2];
    if (!group.hovering && Date.now() - lastFetch >= conf.refreshMs) load();
  });

  onMount(load);

  const unit = $derived(live.settings?.temperature_unit ?? "celsius");
  const ready = $derived(live.device.status === "ready");
  const latest = $derived(ready ? live.latest : null);

  const alt = $derived(
    latest?.pressure && live.settings ? altitude(latest.pressure, live.settings.sea_level_pressure_pa) : null,
  );

  const panels = $derived(layout.order.filter((id) => id !== "relay" || live.relay));
  let innerHeight = $state(800);
  /** Chart height of a maximized card: most of the window. */
  const tallChart = $derived(Math.min(700, Math.max(240, innerHeight - 380)));

  const device = $derived(
    live.device.status === "ready" || live.device.status === "no_module" ? live.device.device : null,
  );
</script>

<svelte:window
  bind:innerHeight
  onkeydown={(e) => {
    if (e.key === "Escape" && layout.maximized) layout.toggleMaximize(layout.maximized);
  }}
/>

{#snippet deviceCard()}
  <section class="card device">
    <div class="device-head">
      <h2><Icon name="plug" size={16} /> {t().overview.device}</h2>
      <div class="actions">
        <button
          class="btn"
          disabled={!device || device.backend !== "hid" || deviceActions.busy !== null}
          title={t().overview.identifyHint}
          aria-busy={deviceActions.busy === "identify"}
          onclick={() => deviceActions.identify()}
        >
          <Icon name={deviceActions.busy === "identify" ? "loader" : "bulb"} size={14} />
          {deviceActions.busy === "identify" ? t().overview.identifying : t().overview.identify}
        </button>
        <button
          class="btn"
          disabled={deviceActions.busy !== null}
          aria-busy={deviceActions.busy === "rescan"}
          onclick={() => deviceActions.rescan()}
        >
          <Icon name={deviceActions.busy === "rescan" ? "loader" : "refresh"} size={14} />
          {deviceActions.busy === "rescan" ? t().overview.rescanning : t().overview.rescan}
        </button>
        <DragHandle onstart={(e) => layout.startDrag(e, "device")} />
      </div>
    </div>
    <p class="action-result" class:error={deviceActions.result && !deviceActions.result.ok} role="status" aria-live="polite">
      {#if deviceActions.result}
        <Icon name={deviceActions.result.ok ? "check" : "warning"} size={14} />
        {deviceActions.result.text}
      {/if}
    </p>
    <dl>
      <div>
        <dt>{t().overview.serial}</dt>
        <dd class="mono">{device?.serial ?? "—"}</dd>
      </div>
      <div>
        <dt>{t().overview.module}</dt>
        <dd>
          {live.device.status === "ready" ? fill(t().overview.climateModule, { revision: live.device.revision }) : "—"}
        </dd>
      </div>
      <div>
        <dt>{t().overview.channel}</dt>
        <dd>{live.device.status === "ready" ? live.device.channel : "—"}</dd>
      </div>
      <div>
        <dt>{t().overview.connection}</dt>
        <dd>{device ? (device.backend === "hid" ? "USB HID" : "Linux i2c-dev") : "—"}</dd>
      </div>
      <div>
        <dt>{t().overview.interval}</dt>
        <dd>{live.settings ? fmtDuration(live.settings.sample_interval_s * 1000) : "—"}</dd>
      </div>
    </dl>
  </section>
{/snippet}

<div class="view">
  <header class="page-head">
    <h1>{t().overview.title}</h1>
    {#if latest}
      <span class="updated"
        ><span class="muted">{t().overview.lastUpdate}</span>
        <span class="mono">{fmtDateTime(latest.ts, { year: true, seconds: true })}</span></span
      >
    {/if}
  </header>

  <DeviceBanners />

  <div class="toolbar">
    <SegmentedControl
      label={t().chart.span}
      value={span}
      options={SPANS.map((s) => ({ value: s.key, label: t().history.ranges[s.key] }))}
      onchange={setSpan}
    />
    {#if group.zoom}
      <button class="btn" onclick={() => group.reset()}><Icon name="refresh" size={14} /> {t().chart.resetZoom}</button>
    {/if}
    <span class="hint muted">{zoomHint()}</span>
  </div>

  <div class="grid">
    {#each panels as id (id)}
      <div
        class="panel"
        class:wide={id === "relay" || id === "device" || layout.maximized === id}
        class:dragging={layout.dragging === id}
        data-panel={id}
        animate:flip={{ duration: 220 }}
      >
        <div class="lift">
          {#if id === "relay"}
            {#if live.relay}<RelayCard relay={live.relay} ondragstart={(e) => layout.startDrag(e, "relay")} />{/if}
          {:else if id === "device"}
            {@render deviceCard()}
          {:else}
            {@const m = id as Metric}
            <MetricCard
              metric={m}
              value={latest ? latest[m] : null}
              {unit}
              history={shown}
              {group}
              rule={live.settings?.alerts[m]}
              stats={windowStats(shown, m, group.zoom)}
              alert={live.alertFor(m)}
              maximized={layout.maximized === m}
              chartHeight={layout.maximized === m ? tallChart : 120}
              onmaximize={() => layout.toggleMaximize(m)}
              ondragstart={(e) => layout.startDrag(e, m)}
            style={chartStyle(live.settings?.charts, m)}
            source={metricSource(live.device, m)}
            onstyle={(st) => live.save((x) => (x.charts = { ...x.charts, [m]: st }))}
            >
              {#snippet footer()}
                {#if m === "pressure" && alt !== null}
                  <span class="tabular" title={t().overview.altitudeHint}
                    ><Icon name="mountain" size={12} /> ≈ {fmtNumber(alt, 0)} m</span
                  >
                {/if}
              {/snippet}
            </MetricCard>
          {/if}
        </div>
      </div>
    {/each}
  </div>
</div>

<style>
  .updated {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
  }
  /* The value sits in a chip, set apart from its label by more than the font. */
  .updated .mono {
    padding: 2px 8px;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--surface-2);
    font-size: 12.5px;
    font-weight: 500;
    color: var(--text);
  }
  .toolbar {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 10px;
  }
  .hint {
    font-size: 12.5px;
  }
  /* At most two charts side by side; full-width panels span both columns. */
  .grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 14px;
  }
  @media (max-width: 860px) {
    .grid {
      grid-template-columns: minmax(0, 1fr);
    }
  }
  .panel {
    position: relative;
    min-width: 0;
  }
  .panel.wide {
    grid-column: 1 / -1;
  }
  .lift,
  .lift > :global(.card) {
    height: 100%;
  }
  /* While dragging, the card follows the pointer above a placeholder. */
  .panel.dragging {
    z-index: 10;
  }
  .panel.dragging::before {
    content: "";
    position: absolute;
    inset: 0;
    border: 2px dashed var(--border-strong);
    border-radius: var(--radius);
    background: color-mix(in srgb, var(--surface-2) 60%, transparent);
  }
  .panel.dragging .lift {
    position: relative;
    pointer-events: none;
  }
  .panel.dragging .lift > :global(.card) {
    box-shadow: 0 16px 40px rgba(0, 0, 0, 0.22);
  }
  :global(body.dragging-panel) {
    cursor: grabbing;
    user-select: none;
  }
  .device {
    padding: 14px 18px 16px;
  }
  .device-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }
  h2 {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    font-size: 14px;
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .action-result {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 6px;
    min-height: 18px;
    margin: 6px 0 0;
    font-size: 12.5px;
    color: var(--good);
  }
  .action-result.error {
    color: var(--warning);
  }
  dl {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
    gap: 12px 18px;
    margin: 14px 0 0;
  }
  dt {
    font-size: 12px;
    color: var(--text-muted);
  }
  dd {
    margin: 2px 0 0;
    font-weight: 500;
    -webkit-user-select: text;
    user-select: text;
  }
</style>
