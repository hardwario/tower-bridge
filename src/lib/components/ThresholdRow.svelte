<script lang="ts">
  import { NO_VALUE, fmtInput, parseInput } from "../format";
  import { fill, plural, t } from "../i18n/index.svelte";
  import type { AlertRule, Metric, TemperatureUnit } from "../types";
  import { deltaFromDisplay, deltaToDisplay, fromDisplay, roundFor, toDisplay, unitLabel } from "../units";
  import Icon from "./Icon.svelte";
  import Toggle from "./Toggle.svelte";

  let {
    metric,
    rule,
    unit,
    active,
    onchange,
  }: {
    metric: Metric;
    rule: AlertRule;
    unit: TemperatureUnit;
    active: boolean;
    onchange: (r: AlertRule) => void;
  } = $props();

  const COOLDOWNS_S = [300, 900, 1800, 3600, 14400];

  function cooldownLabel(s: number): string {
    return s % 3600 === 0 ? plural(t().units.hours, s / 3600) : plural(t().units.minutes, Math.round(s / 60));
  }

  const u = $derived(unitLabel(metric, unit));
  const show = (v: number | null) => (v === null ? "" : fmtInput(roundFor(metric, toDisplay(metric, v, unit))));

  function setLimit(which: "min" | "max", text: string) {
    const v = text.trim() === "" ? null : parseInput(text);
    if (v !== null && !Number.isFinite(v)) return;
    onchange({ ...rule, [which]: v === null ? null : fromDisplay(metric, v, unit) });
  }

  function setHysteresis(text: string) {
    const v = parseInput(text);
    if (!Number.isFinite(v) || v < 0) return;
    onchange({ ...rule, hysteresis: deltaFromDisplay(metric, v, unit) });
  }
</script>

<div class="row" class:off={!rule.enabled}>
  <div class="head">
    <Toggle
      checked={rule.enabled}
      label={fill(t().alerts.alertsFor, { metric: t().metrics[metric] })}
      onchange={(v) => onchange({ ...rule, enabled: v })}
    />
    <span class="name"><Icon name={metric} size={15} /> {t().metrics[metric]}</span>
    {#if active}<span class="badge"><Icon name="warning" size={12} /> {t().alerts.active}</span>{/if}
  </div>
  <div class="fields">
    <label>
      <span class="muted">{t().alerts.below}</span>
      <input
        class="input num"
        inputmode="decimal"
        placeholder={NO_VALUE}
        value={show(rule.min)}
        disabled={!rule.enabled}
        onchange={(e) => setLimit("min", e.currentTarget.value)}
      />
      <span class="unit muted">{u}</span>
    </label>
    <label>
      <span class="muted">{t().alerts.above}</span>
      <input
        class="input num"
        inputmode="decimal"
        placeholder={NO_VALUE}
        value={show(rule.max)}
        disabled={!rule.enabled}
        onchange={(e) => setLimit("max", e.currentTarget.value)}
      />
      <span class="unit muted">{u}</span>
    </label>
    <label title={t().alerts.hysteresisHint}>
      <span class="muted">{t().alerts.hysteresis}</span>
      <input
        class="input num"
        inputmode="decimal"
        value={fmtInput(roundFor(metric, deltaToDisplay(metric, rule.hysteresis, unit)))}
        disabled={!rule.enabled}
        onchange={(e) => setHysteresis(e.currentTarget.value)}
      />
      <span class="unit muted">{u}</span>
    </label>
    <label title={t().alerts.repeatHint}>
      <span class="muted">{t().alerts.repeatAfter}</span>
      <select
        class="select"
        disabled={!rule.enabled}
        value={rule.cooldown_s}
        onchange={(e) => onchange({ ...rule, cooldown_s: Number(e.currentTarget.value) })}
      >
        {#each COOLDOWNS_S as s (s)}
          <option value={s}>{cooldownLabel(s)}</option>
        {/each}
        {#if !COOLDOWNS_S.includes(rule.cooldown_s)}
          <option value={rule.cooldown_s}>{cooldownLabel(rule.cooldown_s)}</option>
        {/if}
      </select>
    </label>
    <label class="recovery">
      <Toggle
        checked={rule.notify_recovery}
        disabled={!rule.enabled}
        label={t().alerts.notifyRecovery}
        onchange={(v) => onchange({ ...rule, notify_recovery: v })}
      />
      <span class="muted">{t().alerts.notifyRecovery}</span>
    </label>
  </div>
</div>

<style>
  .row {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 14px;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .name {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-weight: 600;
  }
  .badge {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 1px 8px;
    border-radius: 10px;
    background: var(--critical-weak);
    color: var(--critical);
    font-size: 12px;
    font-weight: 600;
  }
  .fields {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 10px 18px;
    padding-left: 46px;
  }
  label {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 13px;
  }
  .input.num {
    width: 76px;
  }
  .unit {
    min-width: 22px;
  }
  .off .fields {
    opacity: 0.6;
  }
</style>
