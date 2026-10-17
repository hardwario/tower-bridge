import { api, errorCode, errorMessage } from "./api";
import { describeModules } from "./source";
import { fill, t } from "./i18n/index.svelte";

type Action = "identify" | "rescan";

/** Identify / Rescan with visible progress and outcome (shared by views). */
class DeviceActions {
  busy = $state<Action | null>(null);
  result = $state<{ ok: boolean; text: string } | null>(null);
  private timer: ReturnType<typeof setTimeout> | undefined;

  private show(ok: boolean, text: string) {
    this.result = { ok, text };
    clearTimeout(this.timer);
    this.timer = setTimeout(() => (this.result = null), 5000);
  }

  private fail(e: unknown) {
    const m = t().overview;
    this.show(false, errorCode(e) === "not_connected" ? m.notConnectedError : fill(m.actionFailed, { error: errorMessage(e) }));
  }

  async identify() {
    if (this.busy) return;
    this.busy = "identify";
    this.result = null;
    try {
      // The blinking LED is the answer; only a failure is worth a message.
      await api.identify();
    } catch (e) {
      this.fail(e);
    } finally {
      this.busy = null;
    }
  }

  async rescan() {
    if (this.busy) return;
    this.busy = "rescan";
    this.result = null;
    try {
      const r = await api.rescan();
      const m = t().overview;
      if (r.status === "found") this.show(true, fill(m.rescanFound, { modules: describeModules(r.sources) }));
      else this.show(false, m.rescanNone);
    } catch (e) {
      this.fail(e);
    } finally {
      this.busy = null;
    }
  }
}

export const deviceActions = new DeviceActions();
