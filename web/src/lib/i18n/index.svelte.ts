import { detectLocale, translate, translateCode } from './core';
import type { Locale, MessageKey, Params } from './core';

export { detectLocale } from './core';
export type { Locale, MessageKey } from './core';

export const LOCALES: readonly Locale[] = ['fr', 'en'];
export const STORAGE_KEY = 'ratiod.locale';

function initialLocale(): Locale {
  let stored: string | null = null;
  try {
    stored = localStorage.getItem(STORAGE_KEY);
  } catch {
    // storage blocked: fall back to the browser language
  }
  return detectLocale(stored, typeof navigator === 'undefined' ? undefined : navigator.language);
}

class I18n {
  locale = $state<Locale>(initialLocale());

  constructor() {
    this.#syncDocument();
  }

  setLocale(locale: Locale): void {
    this.locale = locale;
    try {
      localStorage.setItem(STORAGE_KEY, locale);
    } catch {
      // the choice just won't survive a reload
    }
    this.#syncDocument();
  }

  #syncDocument(): void {
    if (typeof document !== 'undefined') document.documentElement.lang = this.locale;
  }
}

export const i18n = new I18n();

export function setLocale(locale: Locale): void {
  i18n.setLocale(locale);
}

/** Translates `key` in the current locale (reactive: reads the locale state). */
export function t(key: MessageKey, params?: Params): string {
  return translate(i18n.locale, key, params);
}

/** `<prefix>.<code>` in the current locale, or `fallback` when the code is unknown. */
export function tCode(prefix: string, code: string | null | undefined, fallback: string, params?: Params): string {
  return translateCode(i18n.locale, prefix, code, fallback, params);
}
