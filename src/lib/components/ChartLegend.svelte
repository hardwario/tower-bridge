<script lang="ts" module>
  import type { WindowStats } from "../chart-group.svelte";
  import type { ChartColor } from "../types";

  /** One line of a chart with several I²C channels. */
  export interface LegendLine {
    channel: number;
    /** Module measuring it, e.g. "Humidity Tag" (with the channel when two read the same). */
    name: string;
    stats: WindowStats | null;
    color: ChartColor;
    /** Out of the chart; the legend shows it in grey. */
    hidden: boolean;
  }
</script>

<script lang="ts">
  import { NO_VALUE, fmtValue } from "../format";
  import { fill, t } from "../i18n/index.svelte";
  import type { ChartStyle, Metric, TemperatureUnit } from "../types";
  import { unitLabel } from "../units";
  import ColorPicker from "./ColorPicker.svelte";

  /**
   * Minimum, average and maximum of the visible window for each line, with
   * the line's colour picker. A line's name shows or hides it in the chart
   * (one always stays); the column heads toggle the minimum, average and
   * maximum lines for all channels at once.
   */
  let {
    metric,
    unit,
    lines,
    style,
    onchange,
    oncolor,
    ontoggle,
  }: {
    metric: Metric;
    unit: TemperatureUnit;
    lines: LegendLine[];
    style: ChartStyle;
    onchange: (style: ChartStyle) => void;
    oncolor: (channel: number, color: ChartColor) => void;
    /** Show or hide a channel's line. */
    ontoggle: (channel: number, hidden: boolean) => void;
  } = $props();

  const KEYS = ["min", "avg", "max"] as const;
  const flag = { min: "show_min", avg: "show_avg", max: "show_max" } as const;
  const shown = $derived(lines.filter((l) => !l.hidden).length);
</script>

<div class="legend" role="group" aria-label={t().metrics[metric]}>
  <!-- The unit once, above the names, so long values leave room for them. -->
  <span class="unit-head">{unitLabel(metric, unit)}</span>
  {#each KEYS as k (k)}
    {@const on = style[flag[k]]}
    <button
      type="button"
      class="toggle {k}"
      class:on
      aria-pressed={on}
      title={t().chart.toggleLine}
      onclick={() => onchange({ ...style, [flag[k]]: !on })}
    >
      <span class="sample"></span>{t().common[k]}
    </button>
  {/each}
  {#each lines as line (line.channel)}
    <!-- The last line in sight stays: an empty chart says nothing. -->
    {@const last = !line.hidden && shown <= 1}
    <span class="source" class:off={line.hidden}>
      <ColorPicker
        value={line.color}
        label="{t().chart.lineColor}: {line.name}"
        onchange={(c) => oncolor(line.channel, c)}
      />
      <button
        type="button"
        class="name"
        aria-pressed={!line.hidden}
        aria-disabled={last}
        title={`${line.name} · ${fill(t().common.channel, { n: line.channel })}\n${last ? t().chart.keepOneLine : t().chart.toggleLine}`}
        onclick={() => !last && ontoggle(line.channel, !line.hidden)}>{line.name}</button
      >
    </span>
    {#each KEYS as k (k)}
      <span
        class="value tabular"
        class:off={line.hidden}
        aria-label="{line.name}, {t().common[k]}, {unitLabel(metric, unit)}"
        >{line.stats ? fmtValue(metric, line.stats[k], unit) : NO_VALUE}</span
      >
    {/each}
  {/each}
</div>

<style>
  .legend {
    display: grid;
    grid-template-columns: minmax(6.5em, 1fr) repeat(3, auto);
    align-items: center;
    column-gap: 4px;
    width: 100%;
  }
  .toggle {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 2px 8px;
    border: none;
    border-radius: 6px;
    background: none;
    font-size: 10.5px;
    font-weight: 600;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: var(--text-muted);
    cursor: pointer;
  }
  .toggle:hover {
    background: var(--surface-2);
  }
  .toggle.on {
    color: var(--text-2);
  }
  /* A short sample of the line: dashed for min/max, dotted for avg. */
  .sample {
    width: 14px;
    border-top: 2px dashed var(--text-muted);
    opacity: 0.35;
  }
  .avg .sample {
    border-top-style: dotted;
  }
  .on .sample {
    opacity: 1;
  }
  /* The swatch is the line's key here, so it never fades like the header tools. */
  .source :global(.trigger) {
    opacity: 1;
  }
  .source {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    padding-right: 6px;
  }
  .name {
    min-width: 0;
    margin: 0 0 0 -4px;
    padding: 2px 4px;
    border: none;
    border-radius: 5px;
    background: none;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font: inherit;
    font-size: 12.5px;
    font-weight: 500;
    color: var(--text);
    cursor: pointer;
  }
  .name:hover {
    background: var(--surface-2);
  }
  .name[aria-disabled="true"] {
    cursor: default;
  }
  .name[aria-disabled="true"]:hover {
    background: none;
  }
  /* A hidden line: greyed out, swatch included, but still there to bring back. */
  .off .name,
  .value.off {
    color: var(--text-muted);
  }
  .value.off {
    opacity: 0.6;
  }
  .source.off :global(.trigger) {
    filter: grayscale(1);
    opacity: 0.45;
  }
  .value {
    padding: 2px 8px;
    font-size: 13px;
    font-weight: 600;
    white-space: nowrap;
    text-align: right;
  }
  .unit-head {
    padding-left: 2px;
    font-size: 11px;
    color: var(--text-muted);
  }
</style>
