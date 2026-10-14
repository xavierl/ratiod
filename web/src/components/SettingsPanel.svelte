<script lang="ts">
  import { untrack } from 'svelte';
  import { fly } from 'svelte/transition';
  import { getClients, saveConfig } from '../lib/api';
  import { formatClient } from '../lib/format';
  import { app } from '../lib/store.svelte';
  import type { AppConfig } from '../lib/types';
  import Icon from './Icon.svelte';

  let form = $state<AppConfig | null>(null);
  let ratioEnabled = $state(false);
  let ratio = $state(2);
  let clients = $state<string[]>([]);
  let search = $state('');
  let errors = $state<Record<string, string>>({});
  let saving = $state(false);

  const filtered = $derived(
    clients.filter((c) => formatClient(c).toLowerCase().includes(search.trim().toLowerCase())),
  );

  // Reset the form each time the panel opens, not on every snapshot.
  $effect(() => {
    if (!app.settingsOpen) return;
    untrack(() => {
      const config = app.snapshot?.config;
      if (!config) return;
      form = { ...config };
      ratioEnabled = config.uploadRatioTarget > 0;
      ratio = config.uploadRatioTarget > 0 ? config.uploadRatioTarget : 2;
      errors = {};
      search = '';
    });
    getClients()
      .then((list) => (clients = list))
      .catch(() => app.notify('error', 'Impossible de charger la liste des clients'));
  });

  function close() {
    app.settingsOpen = false;
  }

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    if (!form) return;
    saving = true;
    errors = {};
    try {
      const result = await saveConfig({ ...form, uploadRatioTarget: ratioEnabled ? ratio : -1 });
      if (result.ok) {
        app.notify('success', 'Réglages enregistrés');
        close();
      } else {
        errors = { [result.error.field]: result.error.message };
      }
    } catch {
      app.notify('error', 'Enregistrement impossible : ratiod ne répond pas.');
    } finally {
      saving = false;
    }
  }
</script>

<svelte:window
  onkeydown={(e) => {
    if (e.key === 'Escape' && app.settingsOpen) close();
  }}
/>

{#if app.settingsOpen && form}
  <button class="fixed inset-0 z-30 cursor-default bg-zinc-950/40 backdrop-blur-[2px]" aria-label="Fermer les réglages" onclick={close}></button>
  <div
    class="fixed inset-y-0 right-0 z-40 flex w-full max-w-md flex-col border-l border-zinc-200 bg-white shadow-2xl dark:border-zinc-800 dark:bg-zinc-900"
    role="dialog"
    aria-modal="true"
    aria-labelledby="settings-title"
    transition:fly={{ x: 400, duration: 200 }}
  >
    <div class="flex items-center justify-between border-b border-zinc-200 px-5 py-4 dark:border-zinc-800">
      <h2 id="settings-title" class="text-lg font-semibold">Réglages</h2>
      <button class="btn btn-ghost p-2" aria-label="Fermer" onclick={close}><Icon name="x" /></button>
    </div>

    <form id="settings-form" class="flex-1 space-y-6 overflow-y-auto px-5 py-5" onsubmit={submit}>
      <div>
        <p class="label">Débit d'upload simulé (ko/s)</p>
        <div class="grid grid-cols-2 gap-3">
          <div>
            <label class="mb-1 block text-xs text-zinc-500" for="min-rate">Minimum</label>
            <input id="min-rate" class="field" type="number" min="0" step="1" required bind:value={form.minUploadRate} />
          </div>
          <div>
            <label class="mb-1 block text-xs text-zinc-500" for="max-rate">Maximum</label>
            <input id="max-rate" class="field" type="number" min="0" step="1" required bind:value={form.maxUploadRate} />
          </div>
        </div>
        {#if errors.minUploadRate || errors.maxUploadRate}
          <p class="error">{errors.minUploadRate ?? errors.maxUploadRate}</p>
        {/if}
        <p class="mt-1.5 text-xs text-zinc-500">Un débit est tiré au hasard dans cette plage toutes les 20 minutes, puis réparti selon les leechers.</p>
      </div>

      <div>
        <label class="label" for="simultaneous">Torrents seedés en même temps</label>
        <input id="simultaneous" class="field" type="number" min="1" step="1" required bind:value={form.simultaneousSeed} />
        {#if errors.simultaneousSeed}<p class="error">{errors.simultaneousSeed}</p>{/if}
      </div>

      <div>
        <label class="label" for="client-search">Client émulé</label>
        <input id="client-search" class="field mb-2" type="search" placeholder="qBittorrent, Transmission…" bind:value={search} />
        <select class="field" size="7" aria-label="Client émulé" bind:value={form.client}>
          {#each filtered as client (client)}
            <option value={client}>{formatClient(client)}</option>
          {/each}
        </select>
        {#if errors.client}<p class="error">{errors.client}</p>{/if}
      </div>

      <div>
        <label class="flex items-center gap-2 text-sm font-medium">
          <input type="checkbox" class="size-4 accent-emerald-600" bind:checked={ratioEnabled} />
          Archiver un torrent quand il atteint un ratio
        </label>
        <input
          class="field mt-2"
          type="number"
          min="0.1"
          step="0.1"
          aria-label="Ratio cible"
          disabled={!ratioEnabled}
          bind:value={ratio}
        />
        {#if errors.uploadRatioTarget}<p class="error">{errors.uploadRatioTarget}</p>{/if}
      </div>

      <label class="flex items-start gap-2 text-sm">
        <input type="checkbox" class="mt-0.5 size-4 accent-emerald-600" bind:checked={form.keepTorrentWithZeroLeechers} />
        <span>
          <span class="font-medium">Garder les torrents sans leecher</span>
          <span class="block text-zinc-500">Sinon, ils sont archivés dès que le tracker n'annonce plus personne.</span>
        </span>
      </label>

      {#if errors.config}<p class="error">{errors.config}</p>{/if}
    </form>

    <div class="flex justify-end gap-2 border-t border-zinc-200 px-5 py-4 dark:border-zinc-800">
      <button class="btn btn-secondary" type="button" onclick={close}>Annuler</button>
      <button class="btn btn-primary" type="submit" form="settings-form" disabled={saving}>
        {saving ? 'Enregistrement…' : 'Enregistrer'}
      </button>
    </div>
  </div>
{/if}
