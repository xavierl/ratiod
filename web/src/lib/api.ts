import type { AppConfig, ConfigError, Snapshot, UploadResult } from './types';

async function ok(response: Promise<Response>): Promise<Response> {
  const r = await response;
  if (!r.ok) throw new Error(`HTTP ${r.status}`);
  return r;
}

export async function startSeed(): Promise<void> {
  await ok(fetch('/api/seed/start', { method: 'POST' }));
}

export async function stopSeed(): Promise<void> {
  await ok(fetch('/api/seed/stop', { method: 'POST' }));
}

export async function getClients(): Promise<string[]> {
  return (await ok(fetch('/api/clients'))).json();
}

export async function saveConfig(config: AppConfig): Promise<{ ok: true } | { ok: false; error: ConfigError }> {
  const response = await fetch('/api/config', {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(config),
  });
  if (response.status === 400) return { ok: false, error: await response.json() };
  await ok(Promise.resolve(response));
  return { ok: true };
}

export async function uploadTorrents(files: File[]): Promise<UploadResult[]> {
  const form = new FormData();
  for (const file of files) form.append('files', file, file.name);
  return (await ok(fetch('/api/torrents', { method: 'POST', body: form }))).json();
}

export async function deleteTorrent(infoHash: string): Promise<void> {
  await ok(fetch(`/api/torrents/${infoHash}`, { method: 'DELETE' }));
}

/** Live snapshots over SSE; EventSource reconnects by itself. Returns an unsubscribe function. */
export function subscribe(onSnapshot: (s: Snapshot) => void, onConnection: (connected: boolean) => void): () => void {
  const source = new EventSource('/api/events');
  source.addEventListener('open', () => onConnection(true));
  source.addEventListener('error', () => onConnection(false));
  source.addEventListener('snapshot', (event) => {
    onConnection(true);
    onSnapshot(JSON.parse((event as MessageEvent<string>).data));
  });
  return () => source.close();
}
