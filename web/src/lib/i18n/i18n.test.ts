import { describe, expect, it } from 'vitest';
import { detectLocale, pluralCategory, translate, translateCode } from './core';
import { en } from './en';
import { fr } from './fr';

const placeholders = (text: string) => [...text.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort();

describe('dictionaries', () => {
  const frKeys = Object.keys(fr).sort();
  const enKeys = Object.keys(en).sort();

  it('have exactly the same keys', () => {
    expect(enKeys).toEqual(frKeys);
  });

  it('have no empty string', () => {
    for (const [key, value] of [...Object.entries(fr), ...Object.entries(en)]) {
      expect(value.trim(), key).not.toBe('');
    }
  });

  it('use the same placeholders in both languages', () => {
    for (const key of frKeys as (keyof typeof fr)[]) {
      expect(placeholders(en[key]), key).toEqual(placeholders(fr[key]));
    }
  });

  it('give every plural key both forms', () => {
    for (const key of frKeys.filter((k) => /^[^_]+_one$/.test(k))) {
      expect(frKeys).toContain(key.replace(/_one$/, '_other'));
    }
  });
});

describe('detectLocale', () => {
  it('prefers a valid stored choice', () => {
    expect(detectLocale('fr', 'en-US')).toBe('fr');
    expect(detectLocale('en', 'fr-FR')).toBe('en');
  });

  it('ignores an invalid stored value', () => {
    expect(detectLocale('de', 'fr-FR')).toBe('fr');
    expect(detectLocale('', 'en-US')).toBe('en');
  });

  it('follows the browser language otherwise', () => {
    expect(detectLocale(null, 'fr-CA')).toBe('fr');
    expect(detectLocale(null, 'FR')).toBe('fr');
    expect(detectLocale(null, 'en-US')).toBe('en');
    expect(detectLocale(null, 'de')).toBe('en');
    expect(detectLocale(null, undefined)).toBe('en');
  });
});

describe('translation helpers', () => {
  it('interpolates placeholders and keeps unknown ones', () => {
    expect(translate('en', 'torrents.failure', { count: 2, max: 5 })).toBe('Failed 2/5');
    expect(translate('fr', 'torrents.failure', { count: 2 })).toBe('Échec 2/{max}');
  });

  it('chooses plural forms per locale', () => {
    expect(pluralCategory('fr', 0)).toBe('one');
    expect(pluralCategory('fr', 1)).toBe('one');
    expect(pluralCategory('fr', 2)).toBe('other');
    expect(pluralCategory('en', 0)).toBe('other');
    expect(pluralCategory('en', 1)).toBe('one');
  });

  it('translates tracker error codes and falls back to the raw text', () => {
    expect(translateCode('en', 'trackerError', 'http_status', 'raw', { detail: '500' })).toBe(
      'The tracker answered HTTP 500',
    );
    expect(translateCode('fr', 'trackerError', 'tracker_failure', 'raw', { detail: 'nope' })).toBe(
      'Refusé par le tracker : nope',
    );
    expect(translateCode('en', 'trackerError', 'mystery', 'raw')).toBe('raw');
    expect(translateCode('en', 'trackerError', null, 'raw')).toBe('raw');
  });
});
