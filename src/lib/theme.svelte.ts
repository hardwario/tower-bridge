/** Colour scheme: user preference (system/light/dark) and the effective theme. */

export type ThemePref = "system" | "light" | "dark";

const query = typeof window === "undefined" ? null : window.matchMedia("(prefers-color-scheme: dark)");

class ThemeState {
  pref = $state<ThemePref>("system");
  systemDark = $state(query?.matches ?? false);

  constructor() {
    query?.addEventListener("change", (e) => (this.systemDark = e.matches));
  }

  get effective(): "light" | "dark" {
    if (this.pref === "system") return this.systemDark ? "dark" : "light";
    return this.pref;
  }

  /** Apply synchronously so CSS variables are current before dependents re-render. */
  apply(pref: ThemePref) {
    this.pref = pref;
    const root = document.documentElement;
    if (pref === "system") delete root.dataset.theme;
    else root.dataset.theme = pref;
  }
}

export const theme = new ThemeState();

/** Chart colours from CSS custom properties. */
export interface ChartTheme {
  series: string;
  band: string;
  grid: string;
  axis: string;
  muted: string;
  text: string;
  surface: string;
  critical: string;
}

export function chartTheme(): ChartTheme {
  const s = getComputedStyle(document.documentElement);
  const v = (name: string) => s.getPropertyValue(name).trim();
  return {
    series: v("--series"),
    band: v("--series-band"),
    grid: v("--grid"),
    axis: v("--axis"),
    muted: v("--text-muted"),
    text: v("--text"),
    surface: v("--surface"),
    critical: v("--critical"),
  };
}
