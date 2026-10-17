<script lang="ts">
  import { onMount } from "svelte";
  import symbol from "./assets/hardwario-symbol.svg";
  import symbolReversed from "./assets/hardwario-symbol-reversed.svg";
  import { api } from "./lib/api";
  import Icon from "./lib/components/Icon.svelte";
  import PrefsSwitcher from "./lib/components/PrefsSwitcher.svelte";
  import { fill, plural, t } from "./lib/i18n/index.svelte";
  import { theme } from "./lib/theme.svelte";
  import StatusPill from "./lib/components/StatusPill.svelte";
  import { live } from "./lib/live.svelte";
  import { moduleLines, sourcesOf } from "./lib/source";
  import Alerts from "./views/Alerts.svelte";
  import Data from "./views/Data.svelte";
  import Manual from "./views/Manual.svelte";
  import Overview from "./views/Overview.svelte";
  import Settings from "./views/Settings.svelte";

  type Tab = "overview" | "data" | "alerts" | "settings" | "manual";
  const TABS: { key: Tab; icon: string }[] = [
    { key: "overview", icon: "gauge" },
    { key: "data", icon: "table" },
    { key: "alerts", icon: "bell" },
    { key: "settings", icon: "sliders" },
    { key: "manual", icon: "book" },
  ];

  let tab = $state<Tab>("overview");
  let failed = $state<string | null>(null);

  onMount(() => {
    live.init().catch((e) => (failed = String(e)));
  });

  /** Every module, one per line as in the Bridge Module card, without the brand (too long here). */
  const devices = $derived(
    moduleLines(sourcesOf(live.device), live.relay, live.nfc, live.gps).map((m) => ({
      key: `${m.channel}:${m.product}`,
      channel: m.channel,
      name: m.product.replace(/^(TOWER|HARDWARIO) /, "") + (m.revision ? ` ${m.revision}` : ""),
      relay: m.product.endsWith("Relay Module"),
    })),
  );
</script>

<div class="app">
  <aside class="sidebar">
    <a
      class="brand"
      href="https://hardwario.academy/"
      title={t().nav.academy}
      aria-label={t().nav.academy}
      onclick={(e) => {
        e.preventDefault();
        api.openLink("academy");
      }}
    >
      <img src={theme.effective === "dark" ? symbolReversed : symbol} alt="" width="22" height="28" />
      <div>
        <div class="name">TOWER Bridge</div>
        <div class="sub">HARDWARIO</div>
      </div>
    </a>
    <nav>
      {#each TABS as tab_ (tab_.key)}
        <button
          data-tab={tab_.key}
          class:active={tab === tab_.key}
          aria-current={tab === tab_.key ? "page" : undefined}
          onclick={() => (tab = tab_.key)}
        >
          <Icon name={tab_.icon} size={16} />
          <span>{t().nav[tab_.key]}</span>
          {#if tab_.key === "alerts" && live.activeAlerts.length}
            <span class="count" aria-label={plural(t().nav.activeAlerts, live.activeAlerts.length)}
              >{live.activeAlerts.length}</span
            >
          {/if}
        </button>
      {/each}
    </nav>
    <div class="bottom">
      <PrefsSwitcher />
    </div>
    <div class="status">
      <!-- While searching, unplugged or without sensors: what is wrong. -->
      {#if live.device.status !== "ready"}<StatusPill device={live.device} />{/if}
      {#each devices as d (d.key)}
        <span class="relay-status" title={fill(t().common.channel, { n: d.channel })}>
          <span class="dot"></span>
          <span class="label"
            >{d.name}{#if d.relay && live.relay}{" "}<span class="muted"
                >· {live.relay.state === null
                  ? t().relay.unknown
                  : live.relay.state
                    ? t().relay.stateTrue
                    : t().relay.stateFalse}</span
              >{/if}</span
          >
        </span>
      {/each}
    </div>
    <div class="about">
      <div class="links">
        <a
          href="https://docs.hardwario.com/tower/"
          onclick={(e) => {
            e.preventDefault();
            api.openLink("docs");
          }}>{t().nav.documentation} <Icon name="external" size={11} /></a
        >
        <a
          href="https://www.hardwario.store/"
          onclick={(e) => {
            e.preventDefault();
            api.openLink("store");
          }}>{t().nav.store} <Icon name="external" size={11} /></a
        >
      </div>
      {#if live.app}<div class="version">{fill(t().nav.version, { version: live.app.version })}</div>{/if}
    </div>
  </aside>

  <main>
    {#if failed}
      <div class="center muted">{fill(t().common.backendError, { error: failed })}</div>
    {:else if !live.ready}
      <div class="center muted">{t().common.loading}</div>
    {:else if tab === "overview"}
      <Overview />
    {:else if tab === "data"}
      <Data />
    {:else if tab === "alerts"}
      <Alerts />
    {:else if tab === "manual"}
      <Manual />
    {:else}
      <Settings />
    {/if}
  </main>
</div>

<style>
  .app {
    display: grid;
    grid-template-columns: 208px 1fr;
    height: 100%;
  }
  .sidebar {
    display: flex;
    flex-direction: column;
    gap: 18px;
    padding: 18px 12px 14px;
    background: var(--sidebar);
    border-right: 1px solid var(--border);
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 4px 8px;
    margin: -2px 0;
    border-radius: var(--radius-sm);
    color: inherit;
    text-decoration: none;
  }
  .brand:hover {
    background: var(--accent-weak);
  }
  .brand img {
    display: block;
  }
  .name {
    font-weight: 700;
    font-size: 15px;
    letter-spacing: -0.01em;
  }
  .sub {
    font-size: 10.5px;
    font-weight: 600;
    letter-spacing: 0.12em;
    color: var(--text-muted);
  }
  nav {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  nav button {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 34px;
    padding: 0 10px;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-2);
    font-weight: 500;
    text-align: left;
    cursor: pointer;
  }
  nav button:hover {
    background: var(--accent-weak);
    color: var(--text);
  }
  nav button.active {
    background: var(--surface);
    color: var(--text);
    box-shadow: var(--shadow), inset 0 0 0 1px var(--border);
  }
  nav button.active :global(svg) {
    color: var(--accent);
  }
  nav span:not(.count) {
    flex: 1;
  }
  .count {
    min-width: 18px;
    height: 18px;
    padding: 0 5px;
    border-radius: 9px;
    background: var(--critical);
    color: #fff;
    font-size: 11px;
    font-weight: 700;
    line-height: 18px;
    text-align: center;
  }
  .bottom {
    margin-top: auto;
    padding: 0 2px;
  }
  .status {
    padding: 10px 10px 0;
    border-top: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .relay-status {
    display: flex;
    align-items: center;
    gap: 7px;
    min-width: 0;
    font-size: 12.5px;
    font-weight: 500;
    color: var(--text-2);
    white-space: nowrap;
  }
  /* Long labels end with an ellipsis instead of squeezing the dot. */
  .relay-status .label {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .relay-status .dot {
    flex-shrink: 0;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--good);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--good) 20%, transparent);
  }
  .relay-status .muted {
    font-weight: 400;
  }
  .about {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 10px 10px 0;
    border-top: 1px solid var(--border);
    font-size: 12px;
  }
  .links {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 12px;
  }
  .links a {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    color: var(--text-2);
    text-decoration: none;
  }
  .links a:hover {
    color: var(--accent);
    text-decoration: underline;
  }
  .version {
    color: var(--text-muted);
    font-size: 11.5px;
  }
  main {
    overflow-y: auto;
    min-width: 0;
  }
  main :global(.view) {
    display: flex;
    flex-direction: column;
    gap: 16px;
    max-width: 1080px;
    margin: 0 auto;
    padding: 22px 26px 32px;
  }
  main :global(.page-head) {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
  }
  main :global(.page-head h1) {
    font-size: 22px;
    letter-spacing: -0.01em;
  }
  .center {
    display: grid;
    place-items: center;
    height: 100%;
  }
</style>
