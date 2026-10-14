const UNITS = ['o', 'ko', 'Mo', 'Go', 'To'];

const CLIENT_NAMES: Record<string, string> = {
  bittorrent: 'BitTorrent',
  deluge: 'Deluge',
  leap: 'Vuze Leap',
  qbittorrent: 'qBittorrent',
  rtorrent: 'rTorrent',
  transmission: 'Transmission',
  utorrent: 'µTorrent',
  vuze: 'Vuze',
};

function decimal(value: number, digits: number): string {
  return value.toLocaleString('fr-FR', { minimumFractionDigits: digits, maximumFractionDigits: digits });
}

/** Decimal units (1 ko = 1000 o), like the backend and Joal. */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return '0 o';
  let value = bytes;
  let unit = 0;
  while (value >= 1000 && unit < UNITS.length - 1) {
    value /= 1000;
    unit++;
  }
  return `${decimal(value, unit === 0 || value >= 100 ? 0 : 1)} ${UNITS[unit]}`;
}

export function formatSpeed(bytesPerSecond: number): string {
  return `${formatBytes(bytesPerSecond)}/s`;
}

export function formatDuration(seconds: number | null): string {
  if (seconds === null) return '—';
  const s = Math.max(0, Math.round(seconds));
  if (s < 60) return `${s} s`;
  if (s < 3600) return `${Math.floor(s / 60)} min`;
  const hours = Math.floor(s / 3600);
  const minutes = Math.floor((s % 3600) / 60);
  return minutes === 0 ? `${hours} h` : `${hours} h ${minutes}`;
}

/** `utorrent-3.5.4_44498.client` → `µTorrent 3.5.4 (44498)`. */
export function formatClient(file: string): string {
  const stem = file.replace(/\.client$/, '');
  const dash = stem.indexOf('-');
  if (dash < 0) return stem;
  const name = CLIENT_NAMES[stem.slice(0, dash)] ?? stem.slice(0, dash);
  const [version, build] = stem.slice(dash + 1).split('_');
  return build ? `${name} ${version} (${build})` : `${name} ${version}`;
}
