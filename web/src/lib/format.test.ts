import { describe, expect, it } from 'vitest';
import { formatBytes, formatClient, formatDuration, formatSpeed } from './format';

describe('formatBytes', () => {
  it('uses decimal French units', () => {
    expect(formatBytes(0)).toBe('0 o');
    expect(formatBytes(999)).toBe('999 o');
    expect(formatBytes(1500)).toBe('1,5 ko');
    expect(formatBytes(142_000)).toBe('142 ko');
    expect(formatBytes(3_800_000_000)).toBe('3,8 Go');
  });

  it('treats junk as zero', () => {
    expect(formatBytes(-5)).toBe('0 o');
    expect(formatBytes(Number.NaN)).toBe('0 o');
  });
});

describe('formatSpeed', () => {
  it('appends /s', () => expect(formatSpeed(142_000)).toBe('142 ko/s'));
});

describe('formatDuration', () => {
  it('formats a countdown', () => {
    expect(formatDuration(null)).toBe('—');
    expect(formatDuration(5)).toBe('5 s');
    expect(formatDuration(90)).toBe('1 min');
    expect(formatDuration(3600)).toBe('1 h');
    expect(formatDuration(3660)).toBe('1 h 1');
  });
});

describe('formatClient', () => {
  it('turns a profile file into a readable name', () => {
    expect(formatClient('qbittorrent-5.2.4.client')).toBe('qBittorrent 5.2.4');
    expect(formatClient('utorrent-3.5.4_44498.client')).toBe('µTorrent 3.5.4 (44498)');
    expect(formatClient('custom.client')).toBe('custom');
  });
});
