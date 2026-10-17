<script lang="ts">
  /** Compact theme and language controls for the sidebar. */
  import { LANGUAGES, type LanguagePref, fill, languageName, t } from "../i18n/index.svelte";
  import { live } from "../live.svelte";
  import type { ThemePref } from "../theme.svelte";
  import Icon from "./Icon.svelte";

  const THEMES: { value: ThemePref; icon: string }[] = [
    { value: "system", icon: "monitor" },
    { value: "light", icon: "sun" },
    { value: "dark", icon: "moon" },
  ];

  function themeLabel(v: ThemePref): string {
    const s = t().settings;
    return v === "system" ? s.themeSystem : v === "light" ? s.themeLight : s.themeDark;
  }
</script>

{#if live.settings}
  {@const s = live.settings}
  <div class="prefs">
    <div class="label" id="prefs-theme-label">{t().settings.appearance}</div>
    <div class="themes" role="radiogroup" aria-labelledby="prefs-theme-label">
      {#each THEMES as th (th.value)}
        <button
          type="button"
          role="radio"
          aria-checked={s.theme === th.value}
          aria-label={themeLabel(th.value)}
          title={themeLabel(th.value)}
          class:active={s.theme === th.value}
          onclick={() => live.save((x) => (x.theme = th.value))}
        >
          <Icon name={th.icon} size={14} />
        </button>
      {/each}
    </div>
    <div class="label" id="prefs-lang-label">{t().settings.language}</div>
    <label class="lang" title={t().settings.language}>
      <span class="icon left"><Icon name="globe" size={14} /></span>
      <span class="icon right"><Icon name="chevron" size={14} /></span>
      <select
        aria-labelledby="prefs-lang-label"
        value={s.language}
        onchange={(e) => {
          const v = e.currentTarget.value as LanguagePref;
          live.save((x) => (x.language = v));
        }}
      >
        <option value="system"
          >{fill(t().settings.languageSystem, { name: languageName(live.app?.system_language ?? "en") })}</option
        >
        {#each LANGUAGES as l (l.code)}
          <option value={l.code} lang={l.code}>{l.name}</option>
        {/each}
      </select>
    </label>
  </div>
{/if}

<style>
  .prefs {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .label {
    padding: 0 2px;
    font-size: 11.5px;
    font-weight: 500;
    color: var(--text-muted);
  }
  .label + .themes,
  .label + .lang {
    margin-bottom: 6px;
  }
  .themes {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 2px;
    padding: 2px;
    border-radius: var(--radius-sm);
    background: var(--surface-2);
    border: 1px solid var(--border);
  }
  .themes button {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 24px;
    border: none;
    border-radius: 6px;
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
  }
  .themes button:hover {
    color: var(--text);
  }
  .themes button.active {
    background: var(--surface);
    color: var(--text);
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.12);
  }
  .lang {
    position: relative;
    display: flex;
    align-items: center;
    color: var(--text-muted);
  }
  .icon {
    position: absolute;
    top: 50%;
    display: flex;
    transform: translateY(-50%);
    pointer-events: none;
  }
  .icon.left {
    left: 8px;
  }
  .icon.right {
    right: 7px;
  }
  .lang select {
    /* Own rendering: the native macOS menu ignores padding and would draw
       the text under the globe icon. */
    -webkit-appearance: none;
    appearance: none;
    width: 100%;
    height: 28px;
    /* Explicit line box: WebKit otherwise sits custom-drawn select text low. */
    line-height: 26px;
    padding: 0 26px 0 28px;
    text-overflow: ellipsis;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface-2);
    color: var(--text-2);
    font-size: 12.5px;
    cursor: pointer;
  }
  .lang select:hover {
    color: var(--text);
  }
</style>
