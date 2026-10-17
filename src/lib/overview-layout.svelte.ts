import { METRICS, type Metric } from "./types";

/** Panels on the overview, in the order the user arranged them. */
export type PanelId = Metric | "relay" | "device";

const DEFAULT_ORDER: PanelId[] = [...METRICS, "relay", "device"];
const KEY = "tower-bridge.overview.layout";

function load(): { order: PanelId[]; maximized: Metric | null } {
  let saved: { order?: unknown; maximized?: unknown } = {};
  try {
    saved = JSON.parse(localStorage.getItem(KEY) ?? "{}");
  } catch {
    /* storage unavailable or corrupt */
  }
  const known = Array.isArray(saved.order)
    ? saved.order.filter((id): id is PanelId => DEFAULT_ORDER.includes(id as PanelId))
    : [];
  // Keep the saved order and append panels it does not know yet.
  const order = [...new Set([...known, ...DEFAULT_ORDER])];
  const maximized = METRICS.includes(saved.maximized as Metric) ? (saved.maximized as Metric) : null;
  return { order, maximized };
}

class OverviewLayout {
  order = $state<PanelId[]>([]);
  maximized = $state<Metric | null>(null);
  /** Panel being dragged. */
  dragging = $state<PanelId | null>(null);

  constructor() {
    const l = load();
    this.order = l.order;
    this.maximized = l.maximized;
  }

  private save() {
    try {
      localStorage.setItem(KEY, JSON.stringify({ order: this.order, maximized: this.maximized }));
    } catch {
      /* storage unavailable */
    }
  }

  /** Move `id` to where `target` is (after it when moving down, before it when moving up). */
  move(id: PanelId, target: PanelId) {
    const from = this.order.indexOf(id);
    const to = this.order.indexOf(target);
    if (from < 0 || to < 0 || from === to) return;
    const next = this.order.filter((p) => p !== id);
    next.splice(to, 0, id);
    this.order = next;
  }

  toggleMaximize(m: Metric) {
    this.maximized = this.maximized === m ? null : m;
    this.save();
  }

  /**
   * Drag a panel by its handle: the panel follows the pointer, a placeholder
   * shows where it will land and the others move aside; on release it settles
   * into the placeholder.
   */
  startDrag(e: PointerEvent, id: PanelId) {
    if (e.button !== 0) return;
    const panel = (e.target as Element).closest<HTMLElement>("[data-panel]");
    const lift = panel?.querySelector<HTMLElement>(":scope > .lift");
    if (!panel || !lift) return;
    e.preventDefault();
    const start = lift.getBoundingClientRect();
    const grab = { x: e.clientX - start.left, y: e.clientY - start.top };
    let pointer = { x: e.clientX, y: e.clientY };
    /** Panel last swapped with; ignored until the pointer leaves it (no ping-pong). */
    let lastTarget: PanelId | null = null;
    const scroller = panel.closest<HTMLElement>("main") ?? document.documentElement;
    this.dragging = id;
    document.body.classList.add("dragging-panel");

    let frame = 0;
    const follow = () => {
      const home = panel.getBoundingClientRect();
      lift.style.transform = `translate(${pointer.x - grab.x - home.left}px, ${pointer.y - grab.y - home.top}px)`;
      // Scroll when the pointer nears the top or bottom edge.
      const box = scroller.getBoundingClientRect();
      if (pointer.y < box.top + 48) scroller.scrollBy(0, -12);
      else if (pointer.y > box.bottom - 48) scroller.scrollBy(0, 12);
      frame = requestAnimationFrame(follow);
    };
    frame = requestAnimationFrame(follow);

    const move = (ev: PointerEvent) => {
      pointer = { x: ev.clientX, y: ev.clientY };
      const over = document.elementFromPoint(ev.clientX, ev.clientY)?.closest<HTMLElement>("[data-panel]");
      const target = over?.dataset.panel as PanelId | undefined;
      if (!target || target === id) {
        lastTarget = null;
      } else if (target !== lastTarget) {
        lastTarget = target;
        this.move(id, target);
      }
    };
    const end = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", end);
      window.removeEventListener("pointercancel", end);
      cancelAnimationFrame(frame);
      document.body.classList.remove("dragging-panel");
      this.save();
      // Settle into the placeholder.
      const from = lift.style.transform;
      lift.style.transform = "";
      const settle = lift.animate([{ transform: from }, { transform: "none" }], {
        duration: 180,
        easing: "cubic-bezier(0.2, 0.8, 0.2, 1)",
      });
      settle.finished.catch(() => {}).finally(() => (this.dragging = null));
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", end);
    window.addEventListener("pointercancel", end);
  }
}

export const layout = new OverviewLayout();
