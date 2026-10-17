import { cs } from "./cs";
import { de } from "./de";
import { en, type Messages } from "./en";
import { pl } from "./pl";
import type { Plural } from "./plural";
import { sk } from "./sk";
import type { DateFormat, TimeFormat } from "../types";

export type Lang = "en" | "cs" | "sk" | "de" | "pl";
export type LanguagePref = "system" | Lang;
export type { Messages, Plural };

/** Languages in the order shown in settings, with their native names. */
export const LANGUAGES: { code: Lang; name: string }[] = [
  { code: "en", name: "English" },
  { code: "cs", name: "Čeština" },
  { code: "sk", name: "Slovenčina" },
  { code: "de", name: "Deutsch" },
  { code: "pl", name: "Polski" },
];

const DICTS: Record<Lang, Messages> = { en, cs, sk, de, pl };

class I18n {
  lang = $state<Lang>("en");
  dateFormat = $state<DateFormat>("auto");
  timeFormat = $state<TimeFormat>("auto");

  get m(): Messages {
    return DICTS[this.lang];
  }

  /** Locale for `Intl` formatting (English follows the system region, e.g. en-GB or en-US). */
  get locale(): string {
    if (this.lang === "en") {
      const nav = typeof navigator === "undefined" ? "" : navigator.language;
      return nav.toLowerCase().startsWith("en") ? nav : "en-GB";
    }
    return this.lang;
  }

  set(lang: Lang) {
    this.lang = lang;
    if (typeof document !== "undefined") document.documentElement.lang = lang;
  }
}

export const i18n = new I18n();

/** Current dictionary. Reading it inside markup or `$derived` is reactive. */
export const t = (): Messages => i18n.m;

/** Replace `{name}` placeholders. */
export function fill(template: string, vars: Record<string, string | number>): string {
  return template.replace(/\{(\w+)\}/g, (match, key: string) => (key in vars ? String(vars[key]) : match));
}

const rulesCache = new Map<string, Intl.PluralRules>();

/** Pick the plural form for `n` and fill `{n}` (locale formatted) and `vars`. */
export function plural(forms: Plural, n: number, vars: Record<string, string | number> = {}): string {
  const locale = i18n.locale;
  let rules = rulesCache.get(locale);
  if (!rules) {
    rules = new Intl.PluralRules(locale);
    rulesCache.set(locale, rules);
  }
  const category = rules.select(n) as keyof Plural;
  const template = forms[category] ?? forms.other;
  return fill(template, { n: new Intl.NumberFormat(locale).format(n), ...vars });
}

export function resolveLang(pref: LanguagePref, system: Lang | undefined): Lang {
  return pref === "system" ? (system ?? "en") : pref;
}

/** Name of a language in its own language. */
export function languageName(code: Lang): string {
  return LANGUAGES.find((l) => l.code === code)?.name ?? code;
}
