<script lang="ts">
  import { onMount } from "svelte";
  import { api } from "../lib/api";
  import Field from "../lib/components/Field.svelte";
  import Icon from "../lib/components/Icon.svelte";
  import Section from "../lib/components/Section.svelte";
  import ThresholdRow from "../lib/components/ThresholdRow.svelte";
  import Toggle from "../lib/components/Toggle.svelte";
  import { fmtDateTime, fmtWithUnit } from "../lib/format";
  import { fill, t } from "../lib/i18n/index.svelte";
  import { live } from "../lib/live.svelte";
  import { channelLabel, metricChannels } from "../lib/source";
  import { METRICS, OPTIONAL_METRICS, type AlertLogEntry } from "../lib/types";

  let log = $state<AlertLogEntry[]>([]);
  let tested = $state(false);

  async function loadLog() {
    try {
      log = await api.alertLog(50);
    } catch (e) {
      console.error(e);
    }
  }

  onMount(loadLog);
  $effect(() => {
    if (live.alertEvents.length) loadLog();
  });

  const unit = $derived(live.settings?.temperature_unit ?? "celsius");

  async function test() {
    await api.testNotification();
    tested = true;
    setTimeout(() => (tested = false), 3000);
  }
</script>

<div class="view">
  <header class="page-head">
    <h1>{t().alerts.title}</h1>
  </header>

  {#if live.settings}
    {@const s = live.settings}
    <Section title={t().alerts.thresholds} description={t().alerts.thresholdsHint}>
      {#each METRICS.filter((m) => !OPTIONAL_METRICS.includes(m) || s.alerts[m].enabled || metricChannels(live.device, m, null, live.latest, false).length) as m (m)}
        <ThresholdRow
          metric={m}
          rule={s.alerts[m]}
          {unit}
          active={!!live.alertFor(m)}
          onchange={(r) => live.save((x) => (x.alerts[m] = r))}
        />
      {/each}
    </Section>

    <Section title={t().alerts.notifications}>
      <Field label={t().alerts.disconnected} hint={t().alerts.disconnectedHint}>
        <Toggle
          checked={s.notify_device_lost}
          label={t().alerts.disconnected}
          onchange={(v) => live.save((x) => (x.notify_device_lost = v))}
        />
      </Field>
      <Field label={t().alerts.test} hint={t().alerts.testHint}>
        <button class="btn" onclick={test}>
          {#if tested}<Icon name="check" size={14} /> {t().alerts.sent}{:else}<Icon name="bell" size={14} />
            {t().alerts.sendTest}{/if}
        </button>
      </Field>
    </Section>

    <Section title={t().alerts.recent}>
      {#if log.length === 0}
        <div class="empty muted">{t().alerts.none}</div>
      {:else}
        {#each log as e (`${e.ts}-${e.metric}-${e.channel}-${e.bound}-${e.kind}`)}
          <div class="entry">
            <span class="kind" class:recovered={e.kind === "recovered"}>
              <Icon name={e.kind === "recovered" ? "check" : "warning"} size={14} />
              {e.kind === "recovered"
                ? t().common.backToNormal
                : e.bound === "high"
                  ? t().common.aboveLimit
                  : t().common.belowLimit}
            </span>
            <span class="what"
              >{t().metrics[e.metric]}
              {#if e.channel !== 0 || metricChannels(live.device, e.metric, null, live.latest).length > 1}
                <span class="muted">({channelLabel(live.device, e.metric, e.channel)})</span>
              {/if}
              {fmtWithUnit(e.metric, e.value, unit)}</span
            >
            <span class="muted">{fill(t().alerts.limit, { value: fmtWithUnit(e.metric, e.threshold, unit) })}</span>
            <span class="muted when tabular">{fmtDateTime(e.ts)}</span>
          </div>
        {/each}
      {/if}
    </Section>
  {/if}
</div>

<style>
  .empty {
    padding: 16px 14px;
  }
  .entry {
    display: grid;
    grid-template-columns: 170px 1fr auto auto;
    align-items: center;
    gap: 12px;
    padding: 9px 14px;
    font-size: 13px;
  }
  .kind {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-weight: 600;
    color: var(--critical);
  }
  .kind.recovered {
    color: var(--good);
  }
  .when {
    min-width: 110px;
    text-align: right;
  }
</style>
