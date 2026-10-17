<script lang="ts">
  import type { Snippet } from "svelte";
  import type { ChartGroup, WindowStats } from "../chart-group.svelte";
  import { fmtValue } from "../format";
  import { t } from "../i18n/index.svelte";
  import { lineColor } from "../palette";
  import { theme } from "../theme.svelte";
  import type { ActiveAlert, AlertRule, ChartStyle, History, Metric, TemperatureUnit } from "../types";
  import { unitLabel } from "../units";
  import ChartStats from "./ChartStats.svelte";
  import ColorPicker from "./ColorPicker.svelte";
  import type { Source } from "../source";
  import DragHandle from "./DragHandle.svelte";
  import ResetView from "./ResetView.svelte";
  import SourceInfo from "./SourceInfo.svelte";
  import Icon from "./Icon.svelte";
  import TimeChart from "./TimeChart.svelte";

  let {
    metric,
    value,
    unit,
    history,
    group,
    rule,
    stats,
    alert,
    footer,
    maximized = false,
    chartHeight = 120,
    onmaximize,
    ondragstart,
    style,
    onstyle,
    source = null,
  }: {
    metric: Metric;
    value: number | null;
    unit: TemperatureUnit;
    history: History | null;
    group: ChartGroup;
    rule?: AlertRule;
    stats: WindowStats | null;
    alert?: ActiveAlert;
    footer?: Snippet;
    maximized?: boolean;
    chartHeight?: number;
    onmaximize?: () => void;
    /** Shows a drag handle for reordering. */
    ondragstart?: (e: PointerEvent) => void;
    style: ChartStyle;
    onstyle: (style: ChartStyle) => void;
    /** Where the values come from, behind the header's info icon. */
    source?: Source | null;
  } = $props();

  const color = $derived(lineColor(style.color, theme.effective === "dark"));
</script>

<article class="card metric" class:alerting={!!alert} class:maximized aria-label={t().metrics[metric]}>
  <header>
    <span class="label"><Icon name={metric} size={15} /> {t().metrics[metric]}</span>
    <span class="tools">
      {#if alert}
        <span class="badge"
          ><Icon name="warning" size={13} /> {alert.bound === "high" ? t().common.aboveLimit : t().common.belowLimit}</span
        >
      {/if}
      <ColorPicker value={style.color} onchange={(c) => onstyle({ ...style, color: c })} />
      {#if source}<SourceInfo {source} />{/if}
      {#if ondragstart}<DragHandle onstart={ondragstart} />{/if}
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
    </span>
  </header>
  <div class="value">
    <span class="number">{fmtValue(metric, value, unit)}</span>
    {#if value !== null}<span class="unit">{unitLabel(metric, unit)}</span>{/if}
  </div>
  <!-- Once panned or zoomed, keep the chart (even over a gap) so it can be dragged back. -->
  {#if history && (history.t.length > 1 || group.zoom)}
    <TimeChart
      {metric}
      {history}
      {unit}
      {group}
      {rule}
      {color}
      show={{ min: style.show_min, avg: style.show_avg, max: style.show_max }}
      height={chartHeight}
      compact={!maximized}
    />
  {:else}
    <div class="placeholder muted" style:height="{chartHeight}px">{t().overview.collecting}</div>
  {/if}
  <footer>
    <ChartStats {metric} {unit} {stats} {style} {color} onchange={onstyle} />
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
  .number {
    font-size: 40px;
    font-weight: 600;
    letter-spacing: -0.02em;
    line-height: 1.05;
  }
  .unit {
    font-size: 18px;
    color: var(--text-2);
    font-weight: 500;
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
  .extra {
    padding-right: 10px;
    font-size: 12.5px;
  }
</style>
