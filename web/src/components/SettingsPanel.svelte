<script lang="ts">
  import { untrack } from 'svelte';
  import { fly } from 'svelte/transition';
  import { getClients, saveConfig } from '../lib/api';
  import { formatClient } from '../lib/format';
  import { t, tCode } from '../lib/i18n/index.svelte';
  import { app } from '../lib/store.svelte';
  import type { AppConfig, ConfigError } from '../lib/types';
  import Icon from './Icon.svelte';

  let form = $state<AppConfig | null>(null);
  let ratioEnabled = $state(false);
  let ratio = $state(2);
  let clients = $state<string[]>([]);
  let search = $state('');
  let errors = $state<Record<string, ConfigError>>({});
  let saving = $state(false);

  /** Translated error of a field (backend text when the code is unknown). */
  const errorOf = (...fields: string[]) => {
    const error = fields.map((f) => errors[f]).find(Boolean);
    return error ? tCode('configError', error.code, error.message) : null;
  };

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
      .catch(() => app.notify('error', t('settings.clientsFailed')));
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
        app.notify('success', t('settings.saved'));
        close();
      } else {
        errors = { [result.error.field]: result.error };
      }
    } catch {
      app.notify('error', t('settings.saveFailed'));
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
  <button class="fixed inset-0 z-30 cursor-default bg-zinc-950/40 backdrop-blur-[2px]" aria-label={t('settings.closePanel')} onclick={close}></button>
  <div
    class="fixed inset-y-0 right-0 z-40 flex w-full max-w-md flex-col border-l border-zinc-200 bg-white shadow-2xl dark:border-zinc-800 dark:bg-zinc-900"
    role="dialog"
    aria-modal="true"
    aria-labelledby="settings-title"
    transition:fly={{ x: 400, duration: 200 }}
  >
    <div class="flex items-center justify-between border-b border-zinc-200 px-5 py-4 dark:border-zinc-800">
      <h2 id="settings-title" class="text-lg font-semibold">{t('settings.title')}</h2>
      <button class="btn btn-ghost p-2" aria-label={t('settings.close')} onclick={close}><Icon name="x" /></button>
    </div>

    <form id="settings-form" class="flex-1 space-y-6 overflow-y-auto px-5 py-5" onsubmit={submit}>
      <div>
        <p class="label">{t('settings.rates')}</p>
        <div class="grid grid-cols-2 gap-3">
          <div>
            <label class="mb-1 block text-xs text-zinc-500" for="min-rate">{t('settings.min')}</label>
            <input id="min-rate" class="field" type="number" min="0" step="1" required bind:value={form.minUploadRate} />
          </div>
          <div>
            <label class="mb-1 block text-xs text-zinc-500" for="max-rate">{t('settings.max')}</label>
            <input id="max-rate" class="field" type="number" min="0" step="1" required bind:value={form.maxUploadRate} />
          </div>
        </div>
        {#if errorOf('minUploadRate', 'maxUploadRate')}
          <p class="error">{errorOf('minUploadRate', 'maxUploadRate')}</p>
        {/if}
        <p class="mt-1.5 text-xs text-zinc-500">{t('settings.ratesHint')}</p>
      </div>

      <div>
        <label class="label" for="simultaneous">{t('settings.simultaneous')}</label>
        <input id="simultaneous" class="field" type="number" min="1" step="1" required bind:value={form.simultaneousSeed} />
        {#if errorOf('simultaneousSeed')}<p class="error">{errorOf('simultaneousSeed')}</p>{/if}
      </div>

      <div>
        <label class="label" for="client-search">{t('settings.client')}</label>
        <input id="client-search" class="field mb-2" type="search" placeholder={t('settings.clientSearch')} bind:value={search} />
        <select class="field" size="7" aria-label={t('settings.client')} bind:value={form.client}>
          {#each filtered as client (client)}
            <option value={client}>{formatClient(client)}</option>
          {/each}
        </select>
        {#if errorOf('client')}<p class="error">{errorOf('client')}</p>{/if}
      </div>

      <div>
        <label class="flex items-center gap-2 text-sm font-medium">
          <input type="checkbox" class="size-4 accent-emerald-600" bind:checked={ratioEnabled} />
          {t('settings.ratioEnable')}
        </label>
        <input
          class="field mt-2"
          type="number"
          min="0.1"
          step="0.1"
          aria-label={t('settings.ratioTarget')}
          disabled={!ratioEnabled}
          bind:value={ratio}
        />
        {#if errorOf('uploadRatioTarget')}<p class="error">{errorOf('uploadRatioTarget')}</p>{/if}
      </div>

      <label class="flex items-start gap-2 text-sm">
        <input type="checkbox" class="mt-0.5 size-4 accent-emerald-600" bind:checked={form.keepTorrentWithZeroLeechers} />
        <span>
          <span class="font-medium">{t('settings.keepZero')}</span>
          <span class="block text-zinc-500">{t('settings.keepZeroHint')}</span>
        </span>
      </label>

      {#if errorOf('config')}<p class="error">{errorOf('config')}</p>{/if}
    </form>

    <div class="flex justify-end gap-2 border-t border-zinc-200 px-5 py-4 dark:border-zinc-800">
      <button class="btn btn-secondary" type="button" onclick={close}>{t('settings.cancel')}</button>
      <button class="btn btn-primary" type="submit" form="settings-form" disabled={saving}>
        {saving ? t('settings.saving') : t('settings.save')}
      </button>
    </div>
  </div>
{/if}
