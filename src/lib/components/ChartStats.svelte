<script lang="ts">
  import type { WindowStats } from "../chart-group.svelte";
  import { NO_VALUE, fmtValue } from "../format";
  import { t } from "../i18n/index.svelte";
  import type { ChartStyle, Metric, TemperatureUnit } from "../types";
  import { unitLabel } from "../units";

  /**
   * Minimum, average and maximum of the visible window. Each is a toggle for
   * its line in the chart and shows that line's style while it is on.
   */
  let {
    metric,
    unit,
    stats,
    style,
    color,
    onchange,
  }: {
    metric: Metric;
    unit: TemperatureUnit;
    stats: WindowStats | null;
    style: ChartStyle;
    /** Line colour, for the min/max samples. */
    color: string;
    onchange: (style: ChartStyle) => void;
  } = $props();

  const KEYS = ["min", "avg", "max"] as const;
  const flag = { min: "show_min", avg: "show_avg", max: "show_max" } as const;
</script>

<div class="stats" role="group" aria-label={t().metrics[metric]}>
  {#each KEYS as k (k)}
    {@const on = style[flag[k]]}
    <button
      type="button"
      class="stat {k}"
      class:on
      aria-pressed={on}
      title={t().chart.toggleLine}
      style:--line={color}
      onclick={() => onchange({ ...style, [flag[k]]: !on })}
    >
      <span class="label"><span class="sample"></span>{t().common[k]}</span>
      <span class="value tabular"
        >{stats ? fmtValue(metric, stats[k], unit) : NO_VALUE}<span class="unit">{unitLabel(metric, unit)}</span></span
      >
    </button>
  {/each}
</div>

<style>
  .stats {
    display: inline-flex;
    align-items: stretch;
  }
  .stat {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 1px;
    padding: 3px 10px;
    border: none;
    border-radius: 6px;
    background: none;
    color: var(--text);
    cursor: pointer;
    text-align: left;
  }
  .stat + .stat {
    position: relative;
  }
  /* Thin dividers between the three values. */
  .stat + .stat::before {
    content: "";
    position: absolute;
    left: 0;
    top: 6px;
    bottom: 6px;
    border-left: 1px solid var(--border);
  }
  .stat:hover {
    background: var(--surface-2);
  }
  .label {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 10.5px;
    font-weight: 600;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: var(--text-muted);
  }
  /* A short sample of the line: dashed in the line colour for min/max, dotted for avg. */
  .sample {
    width: 14px;
    border-top: 2px dashed var(--line);
    opacity: 0.25;
  }
  .avg .sample {
    border-top: 2px dotted var(--text-muted);
  }
  .on .sample {
    opacity: 1;
  }
  .on .label {
    color: var(--text-2);
  }
  .value {
    font-size: 13px;
    font-weight: 600;
    white-space: nowrap;
  }
  .unit {
    margin-left: 3px;
    font-weight: 400;
    color: var(--text-muted);
  }
</style>
