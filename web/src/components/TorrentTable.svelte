<script lang="ts">
  import { deleteTorrent } from '../lib/api';
  import { formatBytes, formatDuration, formatSpeed } from '../lib/format';
  import { i18n, t, tCode } from '../lib/i18n/index.svelte';
  import { app } from '../lib/store.svelte';
  import type { TorrentView } from '../lib/types';
  import Icon from './Icon.svelte';

  /** The engine drops a torrent after this many failed announces in a row. */
  const MAX_FAILS = 5;

  const torrents = $derived(app.snapshot?.torrents ?? []);
  const running = $derived(app.snapshot?.running ?? false);

  async function remove(torrent: TorrentView) {
    if (!confirm(t('torrents.confirmDelete', { name: torrent.name }))) return;
    try {
      await deleteTorrent(torrent.infoHash);
      app.notify('success', t('torrents.deleted'));
    } catch {
      app.notify('error', t('torrents.deleteFailed'));
    }
  }

  const errorText = (torrent: TorrentView) =>
    tCode('trackerError', torrent.lastErrorCode, torrent.lastError ?? '', { detail: torrent.lastErrorDetail ?? '' });
</script>

{#snippet peers(torrent: TorrentView)}
  {#if torrent.consecutiveFails > 0}
    <span class="inline-flex items-center gap-1 text-amber-600 dark:text-amber-400" title={errorText(torrent)}>
      <Icon name="alert" class="size-3.5" />{t('torrents.failure', { count: torrent.consecutiveFails, max: MAX_FAILS })}
    </span>
  {:else if torrent.status === 'starting'}
    <span class="text-zinc-500">{t('torrents.starting')}</span>
  {:else}
    {torrent.seeders ?? '—'} / {torrent.leechers ?? '—'}
  {/if}
{/snippet}

{#snippet removeButton(torrent: TorrentView)}
  <button
    class="btn btn-ghost p-2"
    aria-label={t('torrents.deleteNamed', { name: torrent.name })}
    title={t('torrents.delete')}
    onclick={() => remove(torrent)}
  >
    <Icon name="trash" />
  </button>
{/snippet}

<section class="card overflow-hidden" aria-label={t('torrents.label')}>
  {#if torrents.length === 0}
    <div class="px-6 py-14 text-center">
      <p class="font-medium">{running ? t('torrents.emptyRunning') : t('torrents.emptyStopped')}</p>
      <p class="mt-1 text-sm text-zinc-500">
        {running ? t('torrents.emptyRunningHint') : t('torrents.emptyStoppedHint')}
      </p>
    </div>
  {:else}
    <table class="hidden w-full table-fixed text-sm md:table">
      <thead class="border-b border-zinc-200 text-left text-xs tracking-wide text-zinc-500 uppercase dark:border-zinc-800">
        <tr>
          <th class="w-[38%] px-4 py-3 font-medium">{t('torrents.name')}</th>
          <th class="px-4 py-3 text-right font-medium">{t('torrents.speed')}</th>
          <th class="px-4 py-3 text-right font-medium">{t('torrents.uploaded')}</th>
          <th class="px-4 py-3 text-right font-medium">{t('torrents.peers')}</th>
          <th class="px-4 py-3 text-right font-medium">{t('torrents.nextAnnounce')}</th>
          <th class="w-14"><span class="sr-only">{t('torrents.actions')}</span></th>
        </tr>
      </thead>
      <tbody class="divide-y divide-zinc-100 dark:divide-zinc-800">
        {#each torrents as torrent (torrent.infoHash)}
          <tr class="transition-colors hover:bg-zinc-50 dark:hover:bg-zinc-800/50">
            <td class="truncate px-4 py-3 font-medium" title={torrent.name}>{torrent.name}</td>
            <td class="px-4 py-3 text-right tabular-nums">{formatSpeed(torrent.speed, i18n.locale)}</td>
            <td class="px-4 py-3 text-right tabular-nums">{formatBytes(torrent.uploaded, i18n.locale)}</td>
            <td class="px-4 py-3 text-right tabular-nums">{@render peers(torrent)}</td>
            <td class="px-4 py-3 text-right text-zinc-500 tabular-nums">{formatDuration(torrent.nextAnnounceIn, i18n.locale)}</td>
            <td class="px-2 py-1.5 text-right">{@render removeButton(torrent)}</td>
          </tr>
        {/each}
      </tbody>
    </table>

    <ul class="divide-y divide-zinc-100 md:hidden dark:divide-zinc-800">
      {#each torrents as torrent (torrent.infoHash)}
        <li class="flex items-start gap-2 px-4 py-3">
          <div class="min-w-0 flex-1">
            <p class="truncate font-medium" title={torrent.name}>{torrent.name}</p>
            <p class="mt-1 flex flex-wrap gap-x-3 gap-y-0.5 text-xs text-zinc-500 tabular-nums">
              <span>{formatSpeed(torrent.speed, i18n.locale)}</span>
              <span>{t('torrents.sent', { amount: formatBytes(torrent.uploaded, i18n.locale) })}</span>
              <span>{@render peers(torrent)}</span>
              <span>{t('torrents.announceIn', { time: formatDuration(torrent.nextAnnounceIn, i18n.locale) })}</span>
            </p>
          </div>
          {@render removeButton(torrent)}
        </li>
      {/each}
    </ul>
  {/if}
</section>
