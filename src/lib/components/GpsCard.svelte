<script lang="ts">
  import { api } from "../api";
  import { fmtDateTime, fmtNumber } from "../format";
  import { plural, t } from "../i18n/index.svelte";
  import type { GpsInfo } from "../types";
  import DragHandle from "./DragHandle.svelte";
  import GpsMap from "./GpsMap.svelte";
  import Icon from "./Icon.svelte";

  let { gps, ondragstart }: { gps: GpsInfo; ondragstart?: (e: PointerEvent) => void } = $props();

  const m = $derived(t().gps);
  /** 50.771168° N, 15.056328° E */
  const coordinate = (v: number, pos: string, neg: string) => `${fmtNumber(Math.abs(v), 6)}° ${v >= 0 ? pos : neg}`;
</script>

<section class="card gps">
  <div class="head">
    <h2><Icon name="gps" size={16} /> {m.title}</h2>
    <span class="state" class:on={gps.fix} role="status">
      <span class="dot"></span>{gps.fix ? m.fix : m.noFix}
    </span>
    <span class="muted small">{plural(m.satellites, gps.satellites)}</span>
    <span class="spacer"></span>
    {#if ondragstart}<DragHandle onstart={ondragstart} />{/if}
  </div>

  {#if gps.fix && gps.latitude !== null && gps.longitude !== null}
    {@const lat = gps.latitude}
    {@const lon = gps.longitude}
    <div class="body">
      <div class="info">
        <div class="position">
          <span class="coords mono">{coordinate(lat, "N", "S")}, {coordinate(lon, "E", "W")}</span>
          <button class="btn" type="button" onclick={() => api.openMap(lat, lon)}>
            <Icon name="external" size={13} />
            {m.map}
          </button>
        </div>
        <dl>
          {#if gps.accuracy !== null}
            <div>
              <dt>{m.accuracy}</dt>
              <dd>± {fmtNumber(gps.accuracy, gps.accuracy < 10 ? 1 : 0)} m</dd>
            </div>
          {/if}
          {#if gps.altitude !== null}
            <div>
              <dt>{m.altitude}</dt>
              <dd>{fmtNumber(gps.altitude, 0)} m</dd>
            </div>
          {/if}
          {#if gps.time !== null}
            <div>
              <dt>{m.time}</dt>
              <dd>{fmtDateTime(gps.time, { year: true, seconds: true })}</dd>
            </div>
          {/if}
        </dl>
      </div>
      <div class="map-area"><GpsMap latitude={lat} longitude={lon} accuracy={gps.accuracy} /></div>
    </div>
  {/if}
</section>

<style>
  .gps {
    padding: 16px 18px 14px;
  }
  .head {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px 12px;
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
  .small {
    font-size: 12.5px;
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
  .dot {
    flex-shrink: 0;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    box-shadow: inset 0 0 0 1.5px var(--text-muted);
  }
  /* Green like the module's LED once there is a fix. */
  .state.on .dot {
    background: var(--good);
    box-shadow: none;
  }
  .body {
    display: flex;
    flex-wrap: wrap;
    gap: 16px 24px;
    margin-top: 12px;
  }
  .info {
    flex: 1 1 260px;
  }
  .map-area {
    flex: 2 1 380px;
    min-width: 0;
  }
  .position {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px 16px;
  }
  .coords {
    font-size: 15px;
    font-weight: 500;
    -webkit-user-select: text;
    user-select: text;
  }
  dl {
    display: flex;
    flex-wrap: wrap;
    gap: 12px 32px;
    margin: 16px 0 0;
  }
  dt {
    font-size: 12px;
    color: var(--text-muted);
  }
  dd {
    margin: 2px 0 0;
    font-weight: 500;
  }
</style>
