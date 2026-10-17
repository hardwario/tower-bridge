<script lang="ts">
  import Icon from "../lib/components/Icon.svelte";
  import { deviceActions } from "../lib/device-actions.svelte";
  import Banner from "../lib/components/Banner.svelte";
  import Rich from "../lib/components/Rich.svelte";
  import { fill, t } from "../lib/i18n/index.svelte";
  import { live } from "../lib/live.svelte";

  const d = $derived(live.device);
  const linux = $derived(live.app?.os === "linux");
  const b = $derived(t().banners);
  const udev =
    "sudo cp 70-hardwario-tower-bridge.rules /etc/udev/rules.d/ && sudo udevadm control --reload && sudo udevadm trigger";
</script>

{#if live.app?.simulated}
  <Banner kind="info" title={b.simulationTitle}>{b.simulationBody}</Banner>
{/if}

{#if d.status === "error"}
  {#if d.kind === "not_found"}
    <Banner kind="info" title={b.connectTitle}>{b.connectBody}</Banner>
  {:else if d.kind === "permission_denied"}
    <Banner kind="error" title={b.permissionTitle}>
      {#if linux}
        <Rich text={b.permissionLinux} codes={{ command: udev }} />
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
