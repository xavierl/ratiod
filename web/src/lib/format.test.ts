import { describe, expect, it } from 'vitest';
import { formatBytes, formatClient, formatDuration, formatSpeed } from './format';

describe('formatBytes', () => {
  it('uses decimal French units', () => {
    expect(formatBytes(0, 'fr')).toBe('0 o');
    expect(formatBytes(999, 'fr')).toBe('999 o');
    expect(formatBytes(1500, 'fr')).toBe('1,5 ko');
    expect(formatBytes(142_000, 'fr')).toBe('142 ko');
    expect(formatBytes(3_800_000_000, 'fr')).toBe('3,8 Go');
  });

  it('uses decimal English units', () => {
    expect(formatBytes(0, 'en')).toBe('0 B');
    expect(formatBytes(999, 'en')).toBe('999 B');
    expect(formatBytes(1500, 'en')).toBe('1.5 kB');
    expect(formatBytes(1_500_000, 'en')).toBe('1.5 MB');
    expect(formatBytes(142_000, 'en')).toBe('142 kB');
    expect(formatBytes(3_800_000_000, 'en')).toBe('3.8 GB');
  });

  it('treats junk as zero', () => {
    expect(formatBytes(-5, 'fr')).toBe('0 o');
    expect(formatBytes(Number.NaN, 'en')).toBe('0 B');
  });
});

describe('formatSpeed', () => {
  it('appends /s in both locales', () => {
    expect(formatSpeed(142_000, 'fr')).toBe('142 ko/s');
    expect(formatSpeed(142_000, 'en')).toBe('142 kB/s');
  });
});

describe('formatDuration', () => {
  it('formats a countdown the same way in both locales', () => {
    for (const locale of ['fr', 'en'] as const) {
      expect(formatDuration(null, locale)).toBe('—');
      expect(formatDuration(5, locale)).toBe('5 s');
      expect(formatDuration(90, locale)).toBe('1 min');
      expect(formatDuration(3600, locale)).toBe('1 h');
      expect(formatDuration(3660, locale)).toBe('1 h 1');
    }
  });
});

describe('formatClient', () => {
  it('turns a profile file into a readable name', () => {
    expect(formatClient('qbittorrent-5.2.4.client')).toBe('qBittorrent 5.2.4');
    expect(formatClient('utorrent-3.5.4_44498.client')).toBe('µTorrent 3.5.4 (44498)');
    expect(formatClient('custom.client')).toBe('custom');
  });
});
