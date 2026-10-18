<script lang="ts">
  import { onMount } from "svelte";
  import { flip } from "svelte/animate";
  import { api } from "../lib/api";
  import { ChartGroup, dataExtent, panBounds, windowStats, zoomDetail, zoomHint } from "../lib/chart-group.svelte";
  import { deviceActions } from "../lib/device-actions.svelte";
  import Icon from "../lib/components/Icon.svelte";
  import MetricCard, { type PanelLine } from "../lib/components/MetricCard.svelte";
  import RelayCard from "../lib/components/RelayCard.svelte";
  import NfcCard from "../lib/components/NfcCard.svelte";
  import GpsCard from "../lib/components/GpsCard.svelte";
  import SegmentedControl from "../lib/components/SegmentedControl.svelte";
  import { NO_VALUE, fmtDateTime, fmtDuration, fmtNumber } from "../lib/format";
  import { fill, t } from "../lib/i18n/index.svelte";
  import { live } from "../lib/live.svelte";
  import { layout } from "../lib/overview-layout.svelte";
  import { channelColor, chartStyle, lineColor } from "../lib/palette";
  import { theme } from "../lib/theme.svelte";
  import { channelLabel, channelName, metricChannels, metricSource, moduleLines, sourcesOf } from "../lib/source";
  import { METRICS, OPTIONAL_METRICS, type History, type Metric } from "../lib/types";
  import { altitude } from "../lib/units";
  import { SPANS, type Span } from "../lib/time";
  import DeviceBanners from "./DeviceBanners.svelte";

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
  // Replaced whole, never changed in place: no deep reactivity needed.
  let history = $state.raw<History | null>(null);
  /** Only the newest load may land (an older, slower one must not overwrite it). */
  let loads = 0;
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
    const mine = ++loads;
    try {
      const [h, range] = await Promise.all([api.history(t - conf.ms, t + 1, 240), api.dataRange()]);
      if (mine !== loads) return;
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
  const latest = $derived(ready ? live.latest : []);
  const newest = $derived(ready ? live.newestTs : null);

  /** One line per I²C channel measuring `m` (none for CO₂ and VOC without a sensor or data). */
  function panelLines(m: Metric): PanelLine[] {
    // With the bridge working, only what is connected now: stored data of a
    // module unplugged since is on the Data page (the user, 2026-10-18).
    // Without it, the stored lines, over the whole span so a panel keeps its
    // lines while zooming.
    const fallback = !OPTIONAL_METRICS.includes(m);
    const channels =
      live.device.status === "ready"
        ? metricChannels(live.device, m, null, [], false)
        : metricChannels(live.device, m, [history, shown], latest, fallback);
    return channels.map((channel) => ({
      channel,
      name: channelName(live.device, m, channel),
      value: latest.find((r) => r.channel === channel)?.[m] ?? null,
      stats: windowStats(shown, m, channel, group.zoom),
      source: metricSource(live.device, m, channel),
    }));
  }

  /** Altitude estimated from each barometer, in channel order. */
  const altitudes = $derived(
    live.settings
      ? latest.flatMap((r) =>
          r.pressure === null
            ? []
            : [
                {
                  channel: r.channel,
                  m: altitude(r.pressure, live.settings!.sea_level_pressure_pa),
                  name: channelLabel(live.device, "pressure", r.channel),
                  color: lineColor(
                    channelColor(chartStyle(live.settings!.charts, "pressure"), "pressure", r.channel),
                    theme.effective === "dark",
                  ),
                },
              ],
        )
      : [],
  );
  const modules = $derived(moduleLines(sourcesOf(live.device), live.relay, live.nfc, live.gps));
  /** Measurement intervals offered in the header, in milliseconds (other values set through the API also show). */
  const INTERVALS = [5_000, 10_000, 20_000, 60_000, 180_000];

  const panels = $derived(
    layout.order.filter((id) => {
      if (id === "relay") return !!live.relay;
      if (id === "nfc") return !!live.nfc;
      if (id === "gps") return !!live.gps;
      // A measurement nothing connected measures has no panel.
      if (live.device.status === "ready" || OPTIONAL_METRICS.includes(id as Metric))
        return panelLines(id as Metric).length > 0;
      return true;
    }),
  );
  /** Where the first chart is among the panels; -1 without one. */
  const firstChart = $derived(panels.findIndex((id) => METRICS.includes(id as Metric)));
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
        <!-- Beside the buttons, so a result does not make the card jump. -->
        <span
          class="action-result"
          class:error={deviceActions.result && !deviceActions.result.ok}
          role="status"
          aria-live="polite"
        >
          {#if deviceActions.result}
            <Icon name={deviceActions.result.ok ? "check" : "warning"} size={14} />
            {deviceActions.result.text}
          {/if}
        </span>
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
      </div>
    </div>
    {#if device && device.bridges_connected > 1}
      <p class="several-bridges" role="alert">
        <Icon name="warning" size={14} />
        <span>{fill(t().overview.severalBridges, { identify: t().overview.identify })}</span>
      </p>
    {/if}
    <dl>
      <div class="modules">
        <dt>{t().overview.modules}</dt>
        <!-- One module per line, after its channel: "I²C channel 0: TOWER Climate Module R1". -->
        <dd class="module-list">
          {#each modules as m (`${m.channel}:${m.product}`)}
            <span class="muted">{fill(t().overview.i2cChannel, { n: m.channel })}:</span>
            <span>{m.product}{#if m.revision}{" "}<span class="revision">{m.revision}</span>{/if}</span>
          {:else}
            {NO_VALUE}
          {/each}
        </dd>
      </div>
    </dl>
  </section>
{/snippet}

<div class="view">
  <header class="page-head">
    <h1>{t().overview.title}</h1>
    <div class="head-tools">
      {#if newest !== null}
        <span class="updated"
          ><span class="muted">{t().overview.lastUpdate}</span>
          <span class="mono">{fmtDateTime(newest, { year: true, seconds: true })}</span></span
        >
      {/if}
      {#if live.settings}
        {@const current = live.settings.sample_interval_ms}
        <label class="interval" title={t().overview.intervalHint}>
          <span class="muted">{t().overview.interval}</span>
          <select
            class="select"
            value={current}
            onchange={(e) => live.save((x) => (x.sample_interval_ms = Number(e.currentTarget.value)))}
          >
            {#each INTERVALS.includes(current) ? INTERVALS : [...INTERVALS, current].sort((a, b) => a - b) as ms (ms)}
              <option value={ms}>{fmtDuration(ms)}</option>
            {/each}
          </select>
        </label>
      {/if}
    </div>
  </header>

  <DeviceBanners />

  <div class="toolbar">
    <SegmentedControl
      label={t().chart.span}
      value={span}
      options={SPANS.map((s) => ({ value: s.key, label: t().chart.ranges[s.key] }))}
      onchange={setSpan}
    />
    {#if group.zoom}
      <button class="btn" onclick={() => group.reset()}><Icon name="refresh" size={14} /> {t().chart.resetZoom}</button>
    {/if}
  </div>

  <div class="grid">
    {#each panels as id, i (id)}
      <div
        class="panel"
        style:order={2 * i}
        class:wide={id === "relay" || id === "nfc" || id === "gps" || layout.maximized === id}
        class:dragging={layout.dragging === id}
        data-panel={id}
        animate:flip={{ duration: 220 }}
      >
        <div class="lift">
          {#if id === "relay"}
            {#if live.relay}<RelayCard relay={live.relay} ondragstart={(e) => layout.startDrag(e, "relay")} />{/if}
          {:else if id === "nfc"}
            {#if live.nfc}<NfcCard nfc={live.nfc} ondragstart={(e) => layout.startDrag(e, "nfc")} />{/if}
          {:else if id === "gps"}
            {#if live.gps}<GpsCard gps={live.gps} ondragstart={(e) => layout.startDrag(e, "gps")} />{/if}
          {:else}
            {@const m = id as Metric}
            <MetricCard
              metric={m}
              lines={panelLines(m)}
              {unit}
              history={shown}
              {group}
              rule={live.settings?.alerts[m]}
              alerts={live.alertsFor(m)}
              maximized={layout.maximized === m}
              chartHeight={layout.maximized === m ? tallChart : 120}
              onmaximize={() => layout.toggleMaximize(m)}
              ondragstart={(e) => layout.startDrag(e, m)}
              style={chartStyle(live.settings?.charts, m)}
              onstyle={(st) => live.save((x) => (x.charts = { ...x.charts, [m]: st }))}
            >
              {#snippet footer()}
                {#if m === "pressure" && altitudes.length}
                  <span class="altitudes tabular" title={t().overview.altitudeHint}>
                    <Icon name="mountain" size={12} />
                    {#each altitudes as a (a.channel)}
                      <span class="altitude" title={altitudes.length > 1 ? a.name : undefined}>
                        {#if altitudes.length > 1}<span class="dot" style:background={a.color}></span>{/if}
                        ≈ {fmtNumber(a.m, 0)} m
                      </span>
                    {/each}
                  </span>
                {/if}
              {/snippet}
            </MetricCard>
          {/if}
        </div>
      </div>
    {/each}
    <!-- How to move and zoom the charts, right above the first one (the
         user, 2026-10-18); placed by `order`, as the panels are animated. -->
    {#if firstChart >= 0}
      <p class="hint muted" style:order={2 * firstChart - 1}>{zoomHint()}</p>
    {/if}
    <!-- The TOWER Bridge Module card stays last and does not move. -->
    <div class="panel wide" style:order={2 * panels.length}>
      <div class="lift">{@render deviceCard()}</div>
    </div>
  </div>
</div>

<style>
  .head-tools {
    display: inline-flex;
    align-items: center;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: 10px 18px;
  }
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
    grid-column: 1 / -1;
    margin: 0;
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
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: 8px;
  }
  .action-result {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    margin-right: 4px;
    font-size: 12.5px;
    color: var(--good-text);
  }
  /* Which bridge is read is a guess while several are plugged in. */
  .several-bridges {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    margin: 12px 0 0;
    padding: 8px 12px;
    border-radius: var(--radius-sm);
    background: var(--warning-weak);
    font-size: 13px;
    line-height: 1.45;
  }
  .several-bridges :global(svg) {
    flex-shrink: 0;
    margin-top: 2px;
    color: var(--warning-text);
  }
  .interval {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
  }
  /* As tall as the time chip next to it. */
  .interval .select {
    height: 26px;
    line-height: 24px;
    font-size: 12.5px;
  }

  .action-result.error {
    color: var(--warning-text);
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
  .altitudes {
    display: inline-flex;
    align-items: center;
    gap: 4px 10px;
  }
  .altitude {
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }
  .altitude .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
  }
  /* The module list may be long. */
  .modules {
    grid-column: span 2;
  }
  .module-list {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 2px 8px;
  }
  .module-list .muted {
    font-weight: 400;
  }
  /* The revision matters far less than the product: small and quiet. */
  .revision {
    margin-left: 1px;
    font-size: 0.72em;
    font-weight: 600;
    letter-spacing: 0.03em;
    color: var(--text-muted);
  }
  dd {
    margin: 2px 0 0;
    font-weight: 500;
    -webkit-user-select: text;
    user-select: text;
  }
</style>
