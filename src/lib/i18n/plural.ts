/** Plural forms keyed by `Intl.PluralRules` category; `{n}` is the count. */
export interface Plural {
  one: string;
  few?: string;
  many?: string;
  other: string;
}

export const p = (forms: Plural): Plural => forms;
