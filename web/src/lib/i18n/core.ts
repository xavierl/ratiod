import { en } from './en';
import { fr } from './fr';

export type Locale = 'fr' | 'en';
export type MessageKey = keyof typeof fr;
export type Params = Record<string, string | number>;

const dictionaries: Record<Locale, Record<string, string>> = { fr, en };

export const isLocale = (value: unknown): value is Locale => value === 'fr' || value === 'en';

/** A stored valid choice wins; otherwise French for French browsers, English for everything else. */
export function detectLocale(stored: string | null, navigatorLanguage: string | undefined): Locale {
  if (isLocale(stored)) return stored;
  return navigatorLanguage?.toLowerCase().startsWith('fr') ? 'fr' : 'en';
}

/** Plural category: French uses the singular for 0 and 1, English for 1 only. */
export function pluralCategory(locale: Locale, count: number): 'one' | 'other' {
  return (locale === 'fr' ? count < 2 : count === 1) ? 'one' : 'other';
}

function interpolate(template: string, params?: Params): string {
  if (!params) return template;
  return template.replace(/\{(\w+)\}/g, (match, name: string) => (name in params ? String(params[name]) : match));
}

export function translate(locale: Locale, key: MessageKey, params?: Params): string {
  return interpolate(dictionaries[locale][key], params);
}

/** Plural-aware: picks `<key>_one` or `<key>_other`; `count` is also available as `{count}`. */
export function translateCount(locale: Locale, key: string, count: number, params?: Params): string {
  const full = `${key}_${pluralCategory(locale, count)}`;
  return interpolate(dictionaries[locale][full] ?? full, { count, ...params });
}

/** Translates `<prefix>.<code>` when the dictionary knows it, otherwise returns `fallback` (the backend text). */
export function translateCode(
  locale: Locale,
  prefix: string,
  code: string | null | undefined,
  fallback: string,
  params?: Params,
): string {
  const template = code ? dictionaries[locale][`${prefix}.${code}`] : undefined;
  return template === undefined ? fallback : interpolate(template, params);
}
