<script lang="ts">
  import { onMount } from "svelte";
  import logo from "./assets/hardwario-tower.svg";
  import logoReversed from "./assets/hardwario-tower-reversed.svg";
  import symbol from "./assets/hardwario-symbol.svg";
  import symbolReversed from "./assets/hardwario-symbol-reversed.svg";
  import { api } from "./lib/api";
  import Icon from "./lib/components/Icon.svelte";
  import PrefsSwitcher from "./lib/components/PrefsSwitcher.svelte";
  import { fill, i18n, plural, t } from "./lib/i18n/index.svelte";
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

  /** The sidebar folded to its icons; remembered (the user, 2026-10-18). */
  const COLLAPSED_KEY = "tower-bridge.sidebar.collapsed";
  let collapsed = $state(readCollapsed());

  function readCollapsed(): boolean {
    try {
      return localStorage.getItem(COLLAPSED_KEY) === "1";
    } catch {
      return false;
    }
  }

  function toggleSidebar() {
    collapsed = !collapsed;
    try {
      localStorage.setItem(COLLAPSED_KEY, collapsed ? "1" : "0");
    } catch {
      // Private mode: the sidebar opens expanded next time.
    }
  }

  onMount(() => {
    live.init().catch((e) => (failed = String(e)));
  });

  /** Every module, one per line, without the brand (too long here), A to Z (the user, 2026-10-18). */
  const devices = $derived(
    moduleLines(sourcesOf(live.device), live.relay, live.nfc, live.gps)
      .map((m) => ({
        key: `${m.channel}:${m.product}`,
        channel: m.channel,
        name: m.product.replace(/^(TOWER|HARDWARIO) /, "") + (m.revision ? ` ${m.revision}` : ""),
      }))
      .sort((a, b) => a.name.localeCompare(b.name, i18n.locale) || a.channel - b.channel),
  );
</script>

<div class="app" class:collapsed>
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
      <!-- The HARDWARIO TOWER logo, its symbol alone on the rail (the user, 2026-10-19). -->
      {#if collapsed}
        <img src={theme.effective === "dark" ? symbolReversed : symbol} alt="" width="22" height="28" />
      {:else}
        <img src={theme.effective === "dark" ? logoReversed : logo} alt="" width="132" height="36" />
      {/if}
    </a>
    <nav>
      {#each TABS as tab_ (tab_.key)}
        <button
          data-tab={tab_.key}
          class:active={tab === tab_.key}
          aria-current={tab === tab_.key ? "page" : undefined}
          title={collapsed ? t().nav[tab_.key] : undefined}
          aria-label={collapsed ? t().nav[tab_.key] : undefined}
          onclick={() => (tab = tab_.key)}
        >
          <Icon name={tab_.icon} size={16} />
          {#if !collapsed}<span>{t().nav[tab_.key]}</span>{/if}
          {#if tab_.key === "alerts" && live.activeAlerts.length}
            <span class="count" aria-label={plural(t().nav.activeAlerts, live.activeAlerts.length)}
              >{live.activeAlerts.length}</span
            >
          {/if}
        </button>
      {/each}
    </nav>
    {#if !collapsed}
      <div class="bottom">
        <PrefsSwitcher />
      </div>
      <div class="status">
        <!-- While searching, unplugged or without sensors: what is wrong. -->
        {#if live.device.status !== "ready"}<StatusPill device={live.device} />{/if}
        {#each devices as d (d.key)}
          <!-- Just the modules: the relay state is in its card (the user, 2026-10-18). -->
          <span class="module" title={fill(t().common.channel, { n: d.channel })}>
            <span class="dot"></span>
            <span class="label">{d.name}</span>
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
    {/if}
    <!-- At the very bottom (the user, 2026-10-18): "‹ Collapse", and on the rail "›". -->
    <button
      class="fold"
      type="button"
      title={collapsed ? t().nav.expand : undefined}
      aria-label={collapsed ? t().nav.expand : undefined}
      aria-expanded={!collapsed}
      onclick={toggleSidebar}
    >
      <Icon name={collapsed ? "chevron-right" : "chevron-left"} size={14} />
      {#if !collapsed}<span>{t().nav.collapse}</span>{/if}
    </button>
  </aside>

  <main>
    {#if live.saveError}
      <div class="save-error" role="alert">
        <Icon name="warning" size={14} />
        <span>{fill(t().common.saveFailed, { error: live.saveError })}</span>
        <button class="close" type="button" aria-label={t().manual.close} onclick={() => (live.saveError = null)}>
          <Icon name="close" size={14} />
        </button>
      </div>
    {/if}
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
    /* The grid row must not grow with a long module list. */
    grid-template-rows: minmax(0, 1fr);
  }
  .sidebar {
    display: flex;
    flex-direction: column;
    gap: 14px;
    min-height: 0;
    /* Many modules in a small window: the sidebar scrolls, not the page. */
    overflow-y: auto;
    padding: 16px 12px 12px;
    background: var(--sidebar);
    border-right: 1px solid var(--border);
  }
  .app.collapsed {
    grid-template-columns: 60px 1fr;
  }
  .fold {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-shrink: 0;
    height: 28px;
    margin: -4px 0 0;
    padding: 0 8px;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-muted);
    font-size: 12.5px;
    cursor: pointer;
  }
  .fold:hover {
    background: var(--hover);
    color: var(--text);
  }
  /* On the rail: the arrow alone, at the bottom. */
  .collapsed .fold {
    justify-content: center;
    margin-top: auto;
    padding: 0;
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
    background: var(--hover);
  }
  .brand img {
    display: block;
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
    height: 32px;
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
    background: var(--hover);
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
  /* Folded: the icons alone, the alert count on the bell. */
  .collapsed nav button {
    position: relative;
    justify-content: center;
    padding: 0;
  }
  .collapsed .count {
    position: absolute;
    top: 1px;
    right: 1px;
    min-width: 15px;
    height: 15px;
    padding: 0 3px;
    font-size: 9.5px;
    line-height: 15px;
  }
  .collapsed .sidebar {
    padding: 18px 10px 14px;
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
  .module {
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
  .module .label {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .module .dot {
    flex-shrink: 0;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--good);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--good) 20%, transparent);
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
  /* A failed settings save, above whatever page is open. */
  .save-error {
    position: sticky;
    top: 0;
    z-index: 5;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 14px;
    background: var(--critical-weak);
    color: var(--critical);
    font-size: 13px;
  }
  .save-error span {
    flex: 1;
  }
  .save-error .close {
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: inherit;
    cursor: pointer;
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
