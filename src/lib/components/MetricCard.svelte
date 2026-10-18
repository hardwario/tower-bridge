<script lang="ts" module>
  import type { WindowStats } from "../chart-group.svelte";
  import type { Source } from "../source";

  /** One I²C channel measuring the panel's metric. */
  export interface PanelLine {
    channel: number;
    /** Module measuring it, e.g. "Humidity Tag". */
    name: string;
    value: number | null;
    stats: WindowStats | null;
    source: Source | null;
  }
</script>

<script lang="ts">
  import type { Snippet } from "svelte";
  import type { ChartGroup } from "../chart-group.svelte";
  import { fmtValue } from "../format";
  import { t } from "../i18n/index.svelte";
  import { channelColor, hiddenChannels, lineColor, withChannelColor, withChannelHidden } from "../palette";
  import { lineLabels } from "../source";
  import { theme } from "../theme.svelte";
  import type { ActiveAlert, AlertRule, ChartStyle, History, Metric, TemperatureUnit } from "../types";
  import { unitLabel } from "../units";
  import ChartLegend from "./ChartLegend.svelte";
  import ChartStats from "./ChartStats.svelte";
  import ColorPicker from "./ColorPicker.svelte";
  import DragHandle from "./DragHandle.svelte";
  import ResetView from "./ResetView.svelte";
  import SourceInfo from "./SourceInfo.svelte";
  import Icon from "./Icon.svelte";
  import TimeChart from "./TimeChart.svelte";

  let {
    metric,
    lines,
    unit,
    history,
    group,
    rule,
    alerts = [],
    footer,
    maximized = false,
    chartHeight = 120,
    onmaximize,
    ondragstart,
    style,
    onstyle,
  }: {
    metric: Metric;
    /** At least one; several when sensors on both channels measure the metric. */
    lines: PanelLine[];
    unit: TemperatureUnit;
    history: History | null;
    group: ChartGroup;
    rule?: AlertRule;
    /** Active alerts of this metric, on any channel. */
    alerts?: ActiveAlert[];
    footer?: Snippet;
    maximized?: boolean;
    chartHeight?: number;
    onmaximize?: () => void;
    /** Shows a drag handle for reordering. */
    ondragstart?: (e: PointerEvent) => void;
    style: ChartStyle;
    onstyle: (style: ChartStyle) => void;
  } = $props();

  const dark = $derived(theme.effective === "dark");
  const colorOf = (channel: number) => lineColor(channelColor(style, metric, channel), dark);
  const single = $derived(lines.length === 1 ? lines[0] : null);
  const sources = $derived(lines.flatMap((l) => (l.source ? [l.source] : [])));
  /** Line names, with the channel where two would read the same. */
  const labels = $derived(lineLabels(lines));
  /** Lines measured now; a module unplugged since keeps its line in the
      chart and the legend, but no "-" among the current values. */
  const current = $derived.by(() => {
    const live = lines.filter((l) => l.source !== null || l.value !== null);
    return live.length ? live : lines;
  });
  const labelOf = (channel: number) => labels[lines.findIndex((l) => l.channel === channel)] ?? "";
  /** One badge per direction: one channel may be above while another is below. */
  const bounds = $derived((["high", "low"] as const).filter((b) => alerts.some((a) => a.bound === b)));
  const alerting = (channel: number) => alerts.some((a) => a.channel === channel);
  /** Lines the user took out of the chart (from the legend). */
  const hidden = $derived(hiddenChannels(style, lines.map((l) => l.channel)));
</script>

<article class="card metric" class:alerting={alerts.length > 0} class:maximized aria-label={t().metrics[metric]}>
  <header>
    <span class="label"><Icon name={metric} size={15} /> {t().metrics[metric]}</span>
    <span class="tools">
      {#each bounds as bound (bound)}
        {@const names = alerts.filter((a) => a.bound === bound).map((a) => labelOf(a.channel))}
        <span class="badge" title={lines.length > 1 ? names.join(", ") : undefined}
          ><Icon name="warning" size={13} />
          {bound === "high" ? t().common.aboveLimit : t().common.belowLimit}{#if lines.length > 1}<span
              class="visually-hidden"
              >: {names.join(", ")}</span
            >{/if}</span
        >
      {/each}
      {#if single}
        <ColorPicker
          value={channelColor(style, metric, single.channel)}
          onchange={(c) => onstyle(withChannelColor(style, single.channel, c))}
        />
      {/if}
      {#if sources.length}<SourceInfo {sources} />{/if}
      <ResetView {group} />
      {#if onmaximize}
        <button
          class="maximize"
          title={maximized ? t().overview.restore : t().overview.maximize}
          aria-label={maximized ? t().overview.restore : t().overview.maximize}
          aria-pressed={maximized}
          onclick={onmaximize}><Icon name={maximized ? "minimize" : "maximize"} size={14} /></button
        >
      {/if}
      <!-- Always the rightmost tool, as in every other card. -->
      {#if ondragstart}<DragHandle onstart={ondragstart} />{/if}
    </span>
  </header>
  {#if single}
    <div class="value">
      {#if single.value === null && single.source}
        <!-- Connected, no value yet (a CO2 Module charging, a VOC-LP Tag warming up). -->
        <span class="waiting" title={t().overview.waiting}><Icon name="loader" size={24} /></span>
      {:else}
        <span class="number" class:none={single.value === null}>{fmtValue(metric, single.value, unit)}</span>
      {/if}
      {#if single.value !== null}<span class="unit">{unitLabel(metric, unit)}</span>{/if}
    </div>
  {:else}
    <div class="values">
      {#each current as line (line.channel)}
        <div class="each" class:alerting={alerting(line.channel)}>
          <div class="value">
            {#if line.value === null && line.source}
              <span class="waiting small" title={t().overview.waiting}><Icon name="loader" size={20} /></span>
            {:else}
              <span class="number small" class:none={line.value === null}>{fmtValue(metric, line.value, unit)}</span>
            {/if}
            {#if line.value !== null}<span class="unit">{unitLabel(metric, unit)}</span>{/if}
            {#if alerting(line.channel)}<Icon name="warning" size={14} />{/if}
          </div>
          <div class="who"><span class="dot" style:background={colorOf(line.channel)}></span>{labelOf(line.channel)}</div>
        </div>
      {/each}
    </div>
  {/if}
  <!-- Once panned or zoomed, keep the chart (even over a gap) so it can be dragged back. -->
  {#if history && (history.t.length > 1 || group.zoom)}
    <TimeChart
      {metric}
      {history}
      {unit}
      {group}
      {rule}
      lines={lines.map((l, i) => ({ channel: l.channel, color: colorOf(l.channel), name: labels[i], hidden: hidden.has(l.channel) }))}
      show={{ min: style.show_min, avg: style.show_avg, max: style.show_max }}
      height={chartHeight}
      compact={!maximized}
    />
  {:else}
    <div class="placeholder muted" style:height="{chartHeight}px">{t().overview.collecting}</div>
  {/if}
  <footer class:stacked={!single}>
    {#if single}
      <ChartStats {metric} {unit} stats={single.stats} {style} color={colorOf(single.channel)} onchange={onstyle} />
    {:else}
      <ChartLegend
        {metric}
        {unit}
        {style}
        lines={lines.map((l, i) => ({
          channel: l.channel,
          name: labels[i],
          stats: l.stats,
          color: channelColor(style, metric, l.channel),
          hidden: hidden.has(l.channel),
        }))}
        onchange={onstyle}
        oncolor={(channel, c) => onstyle(withChannelColor(style, channel, c))}
        ontoggle={(channel, off) => onstyle(withChannelHidden(style, channel, off))}
      />
    {/if}
    {#if footer}<span class="extra muted">{@render footer()}</span>{/if}
  </footer>
</article>

<style>
  .metric {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 16px 18px 14px;
    position: relative;
  }
  /* Red top edge while a limit is exceeded; an inset shadow follows the
     rounded corners, so the card need not clip (its popovers may overflow). */
  .metric.alerting {
    box-shadow:
      inset 0 3px 0 var(--critical),
      var(--shadow);
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .label {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--text-2);
    font-weight: 500;
  }
  .tools {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .maximize {
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    padding: 0;
    border: none;
    border-radius: 6px;
    background: none;
    color: var(--text-muted);
    cursor: pointer;
    opacity: 0.5;
    transition: opacity 0.15s;
  }
  .metric:hover .maximize,
  .metric.maximized .maximize,
  .maximize:focus-visible {
    opacity: 1;
  }
  .maximize:hover {
    color: var(--text);
    background: var(--surface-2);
  }
  .badge {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 2px 8px;
    border-radius: 10px;
    background: var(--critical-weak);
    color: var(--critical);
    font-size: 12px;
    font-weight: 600;
  }
  .value {
    display: flex;
    align-items: baseline;
    gap: 6px;
  }
  /* No tightened spacing: at this size it swallowed the decimal point
     ("21.1" read as "211"). Tabular digits keep the unit still. */
  .number {
    font-size: 40px;
    font-weight: 600;
    line-height: 1.05;
    font-variant-numeric: tabular-nums;
  }
  .number.small {
    font-size: 30px;
  }
  /* Waiting for the first value: a quiet spinner the height of a number. */
  .waiting {
    display: inline-flex;
    align-items: center;
    height: 42px;
    color: var(--text-muted);
  }
  .waiting.small {
    height: 32px;
  }
  /* No value: a quiet placeholder, not a bold black bar. */
  .number.none {
    color: var(--text-muted);
  }
  /* A source beyond its limit: its value in the alert colour. */
  .each.alerting .number,
  .each.alerting .value :global(svg) {
    color: var(--critical);
  }
  .visually-hidden {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
  .unit {
    font-size: 18px;
    color: var(--text-2);
    font-weight: 500;
  }
  .values {
    display: flex;
    flex-wrap: wrap;
    gap: 6px 28px;
  }
  .who {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 2px;
    font-size: 12px;
    color: var(--text-muted);
  }
  .dot {
    flex-shrink: 0;
    width: 8px;
    height: 8px;
    border-radius: 50%;
  }
  .placeholder {
    display: grid;
    place-items: center;
    font-size: 12.5px;
  }
  footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    margin: 0 -10px -4px;
  }
  footer.stacked {
    flex-direction: column;
    align-items: stretch;
    margin: 0 -8px -4px;
  }
  footer.stacked .extra {
    align-self: flex-end;
  }
  .extra {
    padding-right: 10px;
    font-size: 12.5px;
  }
</style>
