<script lang="ts">
  import { onDestroy } from "svelte";
  import { api, errorCode, errorMessage } from "../api";
  import { fmtInput, fmtNumber, parseInput } from "../format";
  import { fill, t } from "../i18n/index.svelte";
  import { live } from "../live.svelte";
  import { relaySource } from "../source";
  import type { RelayInfo } from "../types";
  import DragHandle from "./DragHandle.svelte";
  import Icon from "./Icon.svelte";
  import SourceInfo from "./SourceInfo.svelte";

  let { relay, ondragstart }: { relay: RelayInfo; ondragstart?: (e: PointerEvent) => void } = $props();

  const STATES = [true, false];
  const MIN_S = 0.5;
  const MAX_S = 3600;

  let busy = $state(false);
  let error = $state<string | null>(null);
  let now = $state(Date.now());

  const m = $derived(t().relay);
  /** "TRUE" / "FALSE" */
  const name = (state: boolean) => (state ? m.stateTrue : m.stateFalse);
  const source = $derived(relaySource(live.device, relay));
  const pulseS = $derived((live.settings?.relay.pulse_duration_ms ?? 2000) / 1000);
  const remaining = $derived(relay.pulse_until === null ? null : Math.max(0, relay.pulse_until - now));
  /** State the relay holds during a running pulse. */
  const pulsingTo = $derived(remaining !== null ? relay.state : null);

  /** Length of the running pulse as first seen here (it may come from the API). */
  let pulseTotal = $state(1);
  const done = $derived(remaining === null ? 0 : Math.min(100, 100 - (remaining / pulseTotal) * 100));

  // Animate while a pulse runs.
  let frame = 0;
  $effect(() => {
    const until = relay.pulse_until;
    if (until === null) return;
    const start = Date.now();
    now = start;
    pulseTotal = Math.max(1, until - start);
    const tick = () => {
      now = Date.now();
      frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  });
  onDestroy(() => cancelAnimationFrame(frame));

  const seconds = (s: number) => `${fmtNumber(s, s % 1 ? 1 : 0)} s`;

  async function run(call: () => Promise<RelayInfo>) {
    if (busy) return;
    busy = true;
    error = null;
    try {
      live.relay = await call();
    } catch (e) {
      const code = errorCode(e);
      error =
        code === "not_connected"
          ? t().overview.notConnectedError
          : code === "no_relay"
            ? m.gone
            : fill(m.failed, { error: errorMessage(e) });
    } finally {
      busy = false;
    }
  }

  /** Start a pulse, or end the running one early (switching back now). */
  function pulse(to: boolean) {
    if (pulsingTo === to) run(() => api.setRelay(!to));
    else run(() => api.pulseRelay(to));
  }

  function setPulseLength(text: string, input: HTMLInputElement) {
    const s = parseInput(text);
    if (Number.isFinite(s) && s > 0) {
      const value = Math.round(Math.min(MAX_S, Math.max(MIN_S, s)) * 10) / 10;
      live.save((x) => (x.relay.pulse_duration_ms = Math.round(value * 1000)));
      input.value = fmtInput(value);
    } else {
      input.value = fmtInput(pulseS);
    }
  }
</script>

<section class="card relay">
  <div class="head">
    <h2><Icon name="power" size={16} /> {m.title}</h2>
    <span class="state" class:on={relay.state === true} class:off={relay.state === false} role="status">
      <span class="state-label">{m.stateLabel}</span>
      <span class="dot"></span>{relay.state === null ? m.unknown : name(relay.state)}
    </span>
    <span class="spacer"></span>
    {#if source}<SourceInfo sources={[source]} />{/if}
    {#if ondragstart}<DragHandle onstart={ondragstart} />{/if}
  </div>

  <!-- Two rows of a grid, so the four buttons share one width and their edges line up. -->
  <div class="controls">
    <div class="row" role="group" aria-labelledby="relay-manual">
      <span class="row-label" id="relay-manual">{m.manualControl}</span>
      {#each STATES as state (state)}
        <button
          type="button"
          class="btn activate"
          class:current={relay.state === state}
          aria-pressed={relay.state === state}
          disabled={busy || remaining !== null}
          onclick={() => run(() => api.setRelay(state))}
        >
          <span class="dot" class:on={state} class:off={!state}></span>
          {state ? m.activateTrue : m.activateFalse}
        </button>
      {/each}
    </div>

    <div class="row" role="group" aria-labelledby="relay-pulse">
      <span class="row-label" id="relay-pulse">{m.pulseControl}</span>
      {#each STATES as to (to)}
        {@const running = pulsingTo === to}
        {@const hint = running ? m.stopPulse : fill(to ? m.pulseToTrue : m.pulseToFalse, { time: seconds(pulseS) })}
        <button
          type="button"
          class="btn pulse-btn"
          class:running
          style:--done="{running ? done : 0}%"
          title={hint}
          aria-label={hint}
          aria-pressed={running}
          disabled={busy || (remaining !== null && !running)}
          onclick={() => pulse(to)}
        >
          <span class="label"
            ><Icon name={to ? "pulse-high" : "pulse-low"} size={15} />{to ? m.pulseTrue : m.pulseFalse}</span
          >
        </button>
      {/each}
      <label class="length">
        <span class="muted">{m.pulseDuration}</span>
        <input
          class="input num"
          inputmode="decimal"
          value={fmtInput(pulseS)}
          disabled={remaining !== null}
          onchange={(e) => setPulseLength(e.currentTarget.value, e.currentTarget)}
        />
        <span class="muted">s</span>
      </label>
    </div>
  </div>

  {#if error || relay.state === null}
    <p class="detail" class:error={!!error}>
      {#if error}
        <Icon name="warning" size={14} /> {error}
      {:else}
        {m.unknownHint}
      {/if}
    </p>
  {/if}
</section>

<style>
  .relay {
    padding: 16px 18px 14px;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  h2 {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    font-size: 14px;
  }
  .spacer {
    flex: 1;
  }
  .state {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 2px 10px 2px 8px;
    border-radius: 999px;
    background: var(--surface-2);
    border: 1px solid var(--border);
    font-size: 12.5px;
    font-weight: 500;
    color: var(--text-2);
  }
  .state-label {
    color: var(--text-muted);
    font-weight: 400;
  }
  /* Dot in the colour of the module's LED: green TRUE, red FALSE. */
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    box-shadow: inset 0 0 0 1.5px var(--text-muted);
  }
  .state.on .dot {
    background: var(--good);
    box-shadow: none;
  }
  .state.off .dot {
    background: var(--critical);
    box-shadow: none;
  }
  /* An inline grid sizes its 1fr columns to the widest button, so all four
     buttons are equally wide. The rows only group the cells for assistive
     technology. */
  .controls {
    display: inline-grid;
    grid-template-columns: max-content 1fr 1fr max-content;
    align-items: center;
    gap: 8px;
    max-width: 100%;
    margin-top: 14px;
  }
  .row {
    display: contents;
  }
  .row-label {
    grid-column: 1;
    margin-right: 12px;
    font-size: 13px;
    color: var(--text-2);
  }
  .controls .btn {
    justify-content: center;
    min-width: 0;
  }
  /* Narrow windows: the pulse duration gets a line of its own under the buttons. */
  @media (max-width: 1000px) {
    .controls {
      grid-template-columns: max-content 1fr 1fr;
    }
    .length {
      grid-column: 2 / -1;
    }
  }
  /* The button of the current state: border, background and a lit LED, never
     a heavier weight (bolder text is wider, so the button would grow). */
  .activate.current {
    border-color: var(--text-2);
    background: var(--surface-2);
  }
  .dot.on {
    --led: var(--good);
  }
  .dot.off {
    --led: var(--critical);
  }
  .activate.current .dot {
    background: var(--led);
    box-shadow: none;
  }
  /* The running pulse fills its button as time passes. */
  .pulse-btn {
    position: relative;
    overflow: hidden;
  }
  .pulse-btn::before {
    content: "";
    position: absolute;
    inset: 0 auto 0 0;
    width: var(--done);
    background: color-mix(in srgb, var(--series) 22%, transparent);
  }
  .pulse-btn.running {
    border-color: var(--series);
    color: var(--series);
  }
  .pulse-btn .label {
    position: relative;
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .length {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    margin-left: 16px;
    font-size: 13px;
  }
  @media (max-width: 1000px) {
    .length {
      margin-left: 0;
    }
  }
  .length .input {
    width: 56px;
    text-align: right;
  }
  .detail {
    display: flex;
    align-items: center;
    gap: 6px;
    min-height: 18px;
    margin: 10px 0 0;
    font-size: 12.5px;
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }
  .detail.error {
    color: var(--warning-text);
  }
</style>
