<script lang="ts">
  import { api, errorMessage } from "../lib/api";
  import CopyBlock from "../lib/components/CopyBlock.svelte";
  import Field from "../lib/components/Field.svelte";
  import Icon from "../lib/components/Icon.svelte";
  import SegmentedControl from "../lib/components/SegmentedControl.svelte";
  import Section from "../lib/components/Section.svelte";
  import Toggle from "../lib/components/Toggle.svelte";
  import { fmtClock, fmtDate, fmtInput, parseInput } from "../lib/format";
  import { LANGUAGES, type LanguagePref, fill, languageName, plural, t } from "../lib/i18n/index.svelte";
  import { live } from "../lib/live.svelte";
  import type { ThemePref } from "../lib/theme.svelte";
  import {
    DATE_FORMATS,
    TIME_FORMATS,
    type ChannelMode,
    type DateFormat,
    type Metric,
    type TemperatureUnit,
    type TimeFormat,
  } from "../lib/types";

  const INTERVALS = [2, 5, 10, 30, 60, 300];
  const RETENTION = [7, 30, 90, 180, 365, 730, 1825];
  let loginError = $state<string | null>(null);
  let showToken = $state(false);
  let tokenCopied = $state(false);

  async function copyToken(token: string) {
    try {
      await navigator.clipboard.writeText(token);
      tokenCopied = true;
      setTimeout(() => (tokenCopied = false), 2000);
    } catch (e) {
      console.error(e);
    }
  }

  const st = $derived(t().settings);

  /** Today at 14:05, so the examples show the 12/24-hour difference. */
  const sample = new Date().setHours(14, 5, 0, 0);

  function timeLabel(f: TimeFormat): string {
    const example = fmtClock(sample, false, f);
    return fill(f === "h24" ? st.time24 : f === "h12" ? st.time12 : st.formatAuto, { example });
  }

  function intervalLabel(s: number): string {
    return s < 60 ? plural(t().units.seconds, s) : plural(t().units.minutes, s / 60);
  }

  function retentionLabel(d: number): string {
    return d % 365 === 0 ? plural(t().units.years, d / 365) : plural(t().units.days, d);
  }

  async function setLogin(v: boolean) {
    loginError = null;
    try {
      live.openAtLogin = await api.setOpenAtLogin(v);
    } catch (e) {
      loginError = errorMessage(e);
    }
  }

  function setNumber(text: string, apply: (n: number) => void, min: number, max: number) {
    const n = parseInput(text);
    if (Number.isFinite(n) && n >= min && n <= max) apply(n);
  }

  const subscribe = $derived.by(() => {
    const m = live.settings?.mqtt;
    if (!m) return "";
    const auth = m.username ? ` -u ${m.username} -P '…'` : "";
    return `mosquitto_sub -h ${m.allow_lan ? "<this-computer>" : "127.0.0.1"} -p ${m.port}${auth} -t '${m.topic_prefix}/#' -v`;
  });

  function newToken(): string {
    const bytes = crypto.getRandomValues(new Uint8Array(24));
    return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
  }

  const apiUrl = $derived(
    live.api.status === "running" ? live.api.url : `http://127.0.0.1:${live.settings?.api.port ?? 8180}`,
  );

  const snippets = $derived.by(() => {
    const a = live.settings?.api;
    if (!a) return null;
    const auth = a.allow_lan ? ` -H "Authorization: Bearer ${a.token}"` : "";
    const header = a.allow_lan ? ` --header "Authorization: Bearer ${a.token}"` : "";
    const desktop = {
      mcpServers: {
        "tower-bridge": {
          command: live.app?.exe_path ?? "tower-bridge",
          args: ["--mcp-stdio"],
        },
      },
    };
    return {
      curl: `curl${auth} ${apiUrl}/api/v1/readings/latest`,
      claudeCode: `claude mcp add --transport http tower-bridge ${apiUrl}/mcp${header}`,
      claudeDesktop: JSON.stringify(desktop, null, 2),
    };
  });
</script>

<div class="view">
  <header class="page-head">
    <h1>{st.title}</h1>
  </header>

  {#if live.settings}
    {@const s = live.settings}
    <Section title={st.general}>
      <Field label={st.appearance}>
        <SegmentedControl
          label={st.appearance}
          value={s.theme}
          options={[
            { value: "system" as ThemePref, label: st.themeSystem },
            { value: "light" as ThemePref, label: st.themeLight },
            { value: "dark" as ThemePref, label: st.themeDark },
          ]}
          onchange={(v) => live.save((x) => (x.theme = v))}
        />
      </Field>
      <Field label={st.language}>
        <select
          class="select"
          value={s.language}
          onchange={(e) => {
            const v = e.currentTarget.value as LanguagePref;
            live.save((x) => (x.language = v));
          }}
        >
          <option value="system">{fill(st.languageSystem, { name: languageName(live.app?.system_language ?? "en") })}</option>
          {#each LANGUAGES as l (l.code)}
            <option value={l.code} lang={l.code}>{l.name}</option>
          {/each}
        </select>
      </Field>
      <Field label={st.dateFormat}>
        <select
          class="select"
          value={s.date_format}
          onchange={(e) => {
            const v = e.currentTarget.value as DateFormat;
            live.save((x) => (x.date_format = v));
          }}
        >
          {#each DATE_FORMATS as f (f)}
            <option value={f}>{f === "auto" ? fill(st.formatAuto, { example: fmtDate(sample, "auto") }) : fmtDate(sample, f)}</option>
          {/each}
        </select>
      </Field>
      <Field label={st.timeFormat} hint={st.dateTimeHint}>
        <select
          class="select"
          value={s.time_format}
          onchange={(e) => {
            const v = e.currentTarget.value as TimeFormat;
            live.save((x) => (x.time_format = v));
          }}
        >
          {#each TIME_FORMATS as f (f)}
            <option value={f}>{timeLabel(f)}</option>
          {/each}
        </select>
      </Field>
      <Field label={st.temperatureUnit}>
        <SegmentedControl
          label={st.temperatureUnit}
          value={s.temperature_unit}
          options={[
            { value: "celsius" as TemperatureUnit, label: "°C" },
            { value: "fahrenheit" as TemperatureUnit, label: "°F" },
          ]}
          onchange={(v) => live.save((x) => (x.temperature_unit = v))}
        />
      </Field>
      <Field label={live.app?.os === "macos" ? st.valueInMenuBar : st.valueInTray} hint={st.valueHint}>
        <select
          class="select"
          value={s.tray_metric ?? ""}
          onchange={(e) => {
            const v = e.currentTarget.value;
            live.save((x) => (x.tray_metric = v === "" ? null : (v as Metric)));
          }}
        >
          <option value="temperature">{t().metrics.temperature}</option>
          <option value="humidity">{t().metrics.humidity}</option>
          <option value="illuminance">{t().metrics.illuminance}</option>
          <option value="pressure">{t().metrics.pressure}</option>
          <option value="">{st.iconOnly}</option>
        </select>
      </Field>
      <Field label={st.openAtLogin} hint={loginError ?? st.openAtLoginHint}>
        <Toggle checked={live.openAtLogin} label={st.openAtLogin} onchange={setLogin} />
      </Field>
    </Section>

    <Section title={st.measurement}>
      <Field label={st.interval} hint={st.intervalHint}>
        <select
          class="select"
          value={s.sample_interval_s}
          onchange={(e) => live.save((x) => (x.sample_interval_s = Number(e.currentTarget.value)))}
        >
          {#each INTERVALS as i (i)}
            <option value={i}>{intervalLabel(i)}</option>
          {/each}
        </select>
      </Field>
      <Field label={st.seaLevel} hint={st.seaLevelHint}>
        <input
          class="input num"
          inputmode="decimal"
          value={fmtInput(Math.round(s.sea_level_pressure_pa) / 100)}
          onchange={(e) =>
            setNumber(e.currentTarget.value, (n) => live.save((x) => (x.sea_level_pressure_pa = n * 100)), 800, 1100)}
        />
        <span class="muted">hPa</span>
      </Field>
      <Field label={st.channel} hint={st.channelHint}>
        <select
          class="select"
          value={s.i2c_channel}
          onchange={(e) => live.save((x) => (x.i2c_channel = e.currentTarget.value as ChannelMode))}
        >
          <option value="auto">{st.channelAuto}</option>
          <option value="channel0">{fill(st.channelN, { n: 0 })}</option>
          <option value="channel1">{fill(st.channelN, { n: 1 })}</option>
        </select>
      </Field>
    </Section>

    <Section title={st.mqtt.title} description={st.mqtt.description}>
      <Field
        label={st.mqtt.enable}
        hint={live.mqtt.status === "running"
          ? fill(st.mqtt.listening, { address: live.mqtt.address })
          : live.mqtt.status === "error"
            ? `${t().common.error}: ${live.mqtt.message}`
            : live.mqtt.status === "restart_required"
              ? live.mqtt.reason === "stop_broker"
                ? t().banners.restartStopBroker
                : t().banners.restartApplySettings
              : t().common.off}
      >
        {#if live.mqtt.status === "restart_required"}
          <button class="btn primary" onclick={() => api.restart()}>{t().common.restart}</button>
        {/if}
        <Toggle checked={s.mqtt.enabled} label={st.mqtt.enable} onchange={(v) => live.save((x) => (x.mqtt.enabled = v))} />
      </Field>
      <Field label={st.mqtt.port}>
        <input
          class="input num"
          inputmode="numeric"
          value={s.mqtt.port}
          onchange={(e) => setNumber(e.currentTarget.value, (n) => live.save((x) => (x.mqtt.port = Math.round(n))), 1, 65535)}
        />
      </Field>
      <Field label={st.mqtt.allowLan} hint={st.mqtt.allowLanHint}>
        <Toggle checked={s.mqtt.allow_lan} label={st.mqtt.allowLan} onchange={(v) => live.save((x) => (x.mqtt.allow_lan = v))} />
      </Field>
      <Field label={st.mqtt.topicPrefix} hint={st.mqtt.topicPrefixHint}>
        <input
          class="input"
          value={s.mqtt.topic_prefix}
          spellcheck="false"
          onchange={(e) => live.save((x) => (x.mqtt.topic_prefix = e.currentTarget.value))}
        />
      </Field>
      <Field label={st.mqtt.allowControl} hint={fill(st.mqtt.allowControlHint, { prefix: s.mqtt.topic_prefix })}>
        <Toggle
          checked={s.mqtt.allow_control}
          label={st.mqtt.allowControl}
          onchange={(v) => live.save((x) => (x.mqtt.allow_control = v))}
        />
      </Field>
      <Field label={st.mqtt.username} hint={s.mqtt.allow_lan ? st.mqtt.usernameLan : st.mqtt.usernameOptional}>
        <input
          class="input"
          value={s.mqtt.username}
          spellcheck="false"
          autocomplete="off"
          onchange={(e) => live.save((x) => (x.mqtt.username = e.currentTarget.value))}
        />
      </Field>
      <Field label={st.mqtt.password}>
        <input
          class="input"
          type="password"
          value={s.mqtt.password}
          autocomplete="off"
          disabled={!s.mqtt.username}
          onchange={(e) => live.save((x) => (x.mqtt.password = e.currentTarget.value))}
        />
      </Field>
      <div class="topics">
        <div class="muted small">{st.mqtt.topics}</div>
        <pre class="mono">{s.mqtt.topic_prefix}/temperature   °C
{s.mqtt.topic_prefix}/humidity      %
{s.mqtt.topic_prefix}/illuminance   lx
{s.mqtt.topic_prefix}/pressure      Pa
{s.mqtt.topic_prefix}/altitude      m{#if live.relay}
{s.mqtt.topic_prefix}/relay         unknown | left | right{/if}</pre>
        <CopyBlock text={subscribe} />
      </div>
    </Section>

    <Section title={st.api.title} description={st.api.description}>
      <Field
        label={st.api.enable}
        hint={live.api.status === "running"
          ? fill(st.api.listening, { address: live.api.address })
          : live.api.status === "error"
            ? fill(st.api.listenError, { port: live.api.port, message: live.api.message })
            : t().common.off}
      >
        <Toggle checked={s.api.enabled} label={st.api.enable} onchange={(v) => live.save((x) => (x.api.enabled = v))} />
      </Field>
      <Field label={st.api.port}>
        <input
          class="input num"
          inputmode="numeric"
          value={s.api.port}
          onchange={(e) => setNumber(e.currentTarget.value, (n) => live.save((x) => (x.api.port = Math.round(n))), 1, 65535)}
        />
      </Field>
      <Field label={st.api.allowLan} hint={st.api.allowLanHint}>
        <Toggle checked={s.api.allow_lan} label={st.api.allowLan} onchange={(v) => live.save((x) => (x.api.allow_lan = v))} />
      </Field>
      <Field label={st.api.mcp} hint={st.api.mcpHint}>
        <Toggle checked={s.api.mcp} label={st.api.mcp} onchange={(v) => live.save((x) => (x.api.mcp = v))} />
      </Field>
      <Field label={st.api.allowControl} hint={st.api.allowControlHint}>
        <Toggle
          checked={s.api.allow_control}
          label={st.api.allowControl}
          onchange={(v) => live.save((x) => (x.api.allow_control = v))}
        />
      </Field>
      <Field label={st.api.token} hint={st.api.tokenHint}>
        <!-- Same length either way in a fixed-width monospace box, so toggling never reflows. -->
        <code class="token" aria-label={st.api.token}>{showToken ? s.api.token : "•".repeat(s.api.token.length)}</code>
        <button
          class="btn ghost icon"
          aria-pressed={showToken}
          aria-label={showToken ? t().common.hide : t().common.show}
          title={showToken ? t().common.hide : t().common.show}
          onclick={() => (showToken = !showToken)}
        >
          <Icon name={showToken ? "eye-off" : "eye"} size={15} />
        </button>
        <button
          class="btn ghost icon"
          aria-label={t().common.copy}
          title={t().common.copy}
          onclick={() => copyToken(s.api.token)}
        >
          <Icon name={tokenCopied ? "check" : "copy"} size={15} />
        </button>
        <button class="btn" onclick={() => live.save((x) => (x.api.token = newToken()))}>
          <Icon name="refresh" size={14} />
          {st.api.newToken}
        </button>
      </Field>
      {#if s.api.enabled && snippets}
        <div class="topics">
          <CopyBlock label="REST" text={snippets.curl} />
          {#if s.api.mcp}
            <CopyBlock label="Claude Code" text={snippets.claudeCode} />
            <CopyBlock label="Claude Desktop (claude_desktop_config.json)" text={snippets.claudeDesktop} />
          {/if}
        </div>
      {/if}
    </Section>

    <Section title={st.data}>
      <Field label={st.keepHistory}>
        <select
          class="select"
          value={s.retention_days}
          onchange={(e) => live.save((x) => (x.retention_days = Number(e.currentTarget.value)))}
        >
          {#each RETENTION as d (d)}
            <option value={d}>{retentionLabel(d)}</option>
          {/each}
          {#if !RETENTION.includes(s.retention_days)}<option value={s.retention_days}>{retentionLabel(s.retention_days)}</option
            >{/if}
        </select>
      </Field>
      <Field label={st.dataFolder} hint={live.app?.data_dir}>
        {#if live.app?.portable}<span class="badge">{st.portable}</span>{/if}
        <button class="btn" onclick={() => api.openDataFolder()}><Icon name="folder" size={14} /> {t().common.open}</button>
      </Field>
    </Section>

    <Section title={st.about}>
      <div class="about">
        <div><strong>TOWER Bridge</strong> <span class="muted">{fill(st.version, { version: live.app?.version ?? "" })}</span></div>
        <div class="muted small">{st.legal}</div>
      </div>
    </Section>
  {/if}
</div>

<style>
  .topics {
    padding: 12px 14px 14px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  pre {
    margin: 0;
    padding: 10px 12px;
    border-radius: var(--radius-sm);
    background: var(--surface-2);
    overflow-x: auto;
    -webkit-user-select: text;
    user-select: text;
  }
  .token {
    width: 240px;
    flex: none;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--mono);
    font-variant-numeric: tabular-nums;
    padding: 4px 8px;
    border-radius: 6px;
    background: var(--surface-2);
    -webkit-user-select: text;
    user-select: text;
  }
  .btn.icon {
    width: 30px;
    padding: 0;
    justify-content: center;
    flex: none;
  }
  .small {
    font-size: 12.5px;
  }
  .about {
    padding: 14px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .badge {
    padding: 2px 8px;
    border-radius: 10px;
    background: var(--accent-weak);
    color: var(--accent);
    font-size: 12px;
    font-weight: 600;
  }
</style>
