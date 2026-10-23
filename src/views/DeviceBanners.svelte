<script lang="ts">
  import { api } from "../lib/api";
  import Icon from "../lib/components/Icon.svelte";
  import { deviceActions } from "../lib/device-actions.svelte";
  import Banner from "../lib/components/Banner.svelte";
  import CopyBlock from "../lib/components/CopyBlock.svelte";
  import Rich from "../lib/components/Rich.svelte";
  import { fill, t } from "../lib/i18n/index.svelte";
  import { live } from "../lib/live.svelte";

  const d = $derived(live.device);
  const linux = $derived(live.app?.os === "linux");
  const b = $derived(t().banners);
  /** The udev rule of packaging/linux, written in place: the AppImage cannot ship the file. */
  const rule = (subsystem: string) =>
    `'${subsystem}, ATTRS{idVendor}=="0403", ATTRS{idProduct}=="6030", MODE="0660", TAG+="uaccess"'`;
  /** The newer version whose banner was closed; it comes back for the next one. */
  const DISMISSED_KEY = "tower-bridge.update.dismissed";
  let dismissed = $state(readDismissed());
  function readDismissed(): string | null {
    try {
      return localStorage.getItem(DISMISSED_KEY);
    } catch {
      return null;
    }
  }
  function dismiss(version: string) {
    dismissed = version;
    try {
      localStorage.setItem(DISMISSED_KEY, version);
    } catch {
      // Closed for this session only.
    }
  }
  const update = $derived(live.update.available);
  const udev =
    `printf '%s\\n' ${rule('KERNEL=="hidraw*"')} ${rule('SUBSYSTEM=="i2c-dev"')}` +
    " | sudo tee /etc/udev/rules.d/70-hardwario-tower-bridge.rules > /dev/null" +
    " && sudo udevadm control --reload && sudo udevadm trigger";
</script>

{#if update && update.version !== dismissed}
  <Banner kind="info" title={fill(b.updateTitle, { version: update.version })}>
    {b.updateBody}
    {#snippet actions()}
      <button class="btn primary" onclick={() => api.openRelease()}>
        <Icon name="download" size={14} />
        {t().common.download}
      </button>
      <button
        class="btn ghost close"
        aria-label={t().common.close}
        title={t().common.close}
        onclick={() => dismiss(update.version)}
      >
        <Icon name="close" size={15} />
      </button>
    {/snippet}
  </Banner>
{/if}

{#if live.app?.simulated}
  <Banner kind="info" title={b.simulationTitle}>{b.simulationBody}</Banner>
{/if}

{#if d.status === "error"}
  {#if d.kind === "not_found"}
    <Banner kind="info" title={b.connectTitle}>{b.connectBody}</Banner>
  {:else if d.kind === "permission_denied"}
    <Banner kind="error" title={b.permissionTitle}>
      {#if linux}
        {b.permissionLinux}
        <CopyBlock text={udev} />
      {:else}
        {d.message}
      {/if}
    </Banner>
  {:else if d.kind === "i2c_dev_not_loaded"}
    <Banner kind="warning" title={b.i2cDevTitle}>
      <Rich
        text={b.i2cDevBody}
        codes={{ load: "sudo modprobe i2c-dev", persist: "echo i2c-dev | sudo tee /etc/modules-load.d/i2c-dev.conf" }}
      />
    </Banner>
  {:else if d.kind === "busy"}
    <Banner kind="warning" title={b.busyTitle}>{b.busyBody}</Banner>
  {:else}
    <Banner kind="error" title={b.errorTitle}>{fill(b.errorBody, { message: d.message })}</Banner>
  {/if}
{:else if d.status === "no_module"}
  <Banner kind="warning" title={b.noModuleTitle}>
    {b.noModuleBody}
    {#snippet actions()}
      <button
        class="btn"
        disabled={deviceActions.busy !== null}
        aria-busy={deviceActions.busy === "rescan"}
        onclick={() => deviceActions.rescan()}
      >
        {#if deviceActions.busy === "rescan"}<Icon name="loader" size={14} /> {t().overview.rescanning}{:else}{t()
            .common.retry}{/if}
      </button>
    {/snippet}
  </Banner>
{/if}

<style>
  .close {
    width: 30px;
    padding: 0;
    justify-content: center;
  }
</style>
