<script lang="ts">
  import { deleteTorrent } from '../lib/api';
  import { formatBytes, formatDuration, formatSpeed } from '../lib/format';
  import { app } from '../lib/store.svelte';
  import type { TorrentView } from '../lib/types';
  import Icon from './Icon.svelte';

  const torrents = $derived(app.snapshot?.torrents ?? []);
  const running = $derived(app.snapshot?.running ?? false);

  async function remove(t: TorrentView) {
    if (!confirm(`Supprimer « ${t.name} » ? Le fichier .torrent sera effacé.`)) return;
    try {
      await deleteTorrent(t.infoHash);
      app.notify('success', 'Torrent supprimé');
    } catch {
      app.notify('error', 'Suppression impossible');
    }
  }
</script>

{#snippet peers(t: TorrentView)}
  {#if t.consecutiveFails > 0}
    <span class="inline-flex items-center gap-1 text-amber-600 dark:text-amber-400" title={t.lastError ?? ''}>
      <Icon name="alert" class="size-3.5" />Échec {t.consecutiveFails}/5
    </span>
  {:else if t.status === 'starting'}
    <span class="text-zinc-500">Démarrage…</span>
  {:else}
    {t.seeders ?? '—'} / {t.leechers ?? '—'}
  {/if}
{/snippet}

{#snippet removeButton(t: TorrentView)}
  <button class="btn btn-ghost p-2" aria-label={`Supprimer ${t.name}`} title="Supprimer" onclick={() => remove(t)}>
    <Icon name="trash" />
  </button>
{/snippet}

<section class="card overflow-hidden" aria-label="Torrents">
  {#if torrents.length === 0}
    <div class="px-6 py-14 text-center">
      <p class="font-medium">{running ? 'Aucun torrent en cours' : 'Le seed est arrêté'}</p>
      <p class="mt-1 text-sm text-zinc-500">
        {running ? 'Ajoute des fichiers .torrent pour commencer.' : 'Démarre-le pour annoncer tes torrents aux trackers.'}
      </p>
    </div>
  {:else}
    <table class="hidden w-full table-fixed text-sm md:table">
      <thead class="border-b border-zinc-200 text-left text-xs tracking-wide text-zinc-500 uppercase dark:border-zinc-800">
        <tr>
          <th class="w-[38%] px-4 py-3 font-medium">Nom</th>
          <th class="px-4 py-3 text-right font-medium">Débit</th>
          <th class="px-4 py-3 text-right font-medium">Envoyé</th>
          <th class="px-4 py-3 text-right font-medium">Seed / Leech</th>
          <th class="px-4 py-3 text-right font-medium">Prochaine annonce</th>
          <th class="w-14"><span class="sr-only">Actions</span></th>
        </tr>
      </thead>
      <tbody class="divide-y divide-zinc-100 dark:divide-zinc-800">
        {#each torrents as t (t.infoHash)}
          <tr class="transition-colors hover:bg-zinc-50 dark:hover:bg-zinc-800/50">
            <td class="truncate px-4 py-3 font-medium" title={t.name}>{t.name}</td>
            <td class="px-4 py-3 text-right tabular-nums">{formatSpeed(t.speed)}</td>
            <td class="px-4 py-3 text-right tabular-nums">{formatBytes(t.uploaded)}</td>
            <td class="px-4 py-3 text-right tabular-nums">{@render peers(t)}</td>
            <td class="px-4 py-3 text-right text-zinc-500 tabular-nums">{formatDuration(t.nextAnnounceIn)}</td>
            <td class="px-2 py-1.5 text-right">{@render removeButton(t)}</td>
          </tr>
        {/each}
      </tbody>
    </table>

    <ul class="divide-y divide-zinc-100 md:hidden dark:divide-zinc-800">
      {#each torrents as t (t.infoHash)}
        <li class="flex items-start gap-2 px-4 py-3">
          <div class="min-w-0 flex-1">
            <p class="truncate font-medium" title={t.name}>{t.name}</p>
            <p class="mt-1 flex flex-wrap gap-x-3 gap-y-0.5 text-xs text-zinc-500 tabular-nums">
              <span>{formatSpeed(t.speed)}</span>
              <span>{formatBytes(t.uploaded)} envoyés</span>
              <span>{@render peers(t)}</span>
              <span>annonce dans {formatDuration(t.nextAnnounceIn)}</span>
            </p>
          </div>
          {@render removeButton(t)}
        </li>
      {/each}
    </ul>
  {/if}
</section>
