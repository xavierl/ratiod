<script lang="ts">
  import { startSeed, stopSeed } from '../lib/api';
  import { formatClient } from '../lib/format';
  import { app } from '../lib/store.svelte';
  import Icon from './Icon.svelte';

  let pending = $state(false);
  const snapshot = $derived(app.snapshot);
  const running = $derived(snapshot?.running ?? false);
  const badge = $derived(
    !app.connected
      ? { text: 'Connexion perdue', tone: 'bg-amber-100 text-amber-800 dark:bg-amber-950 dark:text-amber-300', dot: 'bg-amber-500' }
      : running
        ? { text: 'Seed actif', tone: 'bg-emerald-100 text-emerald-800 dark:bg-emerald-950 dark:text-emerald-300', dot: 'bg-emerald-500 animate-pulse' }
        : { text: 'Arrêté', tone: 'bg-zinc-200 text-zinc-700 dark:bg-zinc-800 dark:text-zinc-300', dot: 'bg-zinc-400' },
  );

  async function toggle() {
    pending = true;
    try {
      if (running) await stopSeed();
      else await startSeed();
    } catch {
      app.notify('error', 'Action impossible : ratiod ne répond pas.');
    } finally {
      pending = false;
    }
  }
</script>

<header class="flex flex-wrap items-center gap-x-3 gap-y-2">
  <h1 class="text-xl font-semibold tracking-tight">ratiod</h1>
  <span class="inline-flex items-center gap-1.5 rounded-full px-2.5 py-1 text-xs font-medium {badge.tone}">
    <span class="size-1.5 rounded-full {badge.dot}"></span>{badge.text}
  </span>
  {#if snapshot}
    <span class="text-sm text-zinc-500">{formatClient(snapshot.client)}</span>
  {/if}
  <div class="ml-auto flex items-center gap-2">
    <button class="btn {running ? 'btn-secondary' : 'btn-primary'}" onclick={toggle} disabled={!snapshot || pending || !app.connected}>
      <Icon name={running ? 'pause' : 'play'} />
      {running ? 'Arrêter' : 'Démarrer'}
    </button>
    <button class="btn btn-ghost p-2" aria-label="Réglages" onclick={() => (app.settingsOpen = true)} disabled={!snapshot}>
      <Icon name="settings" class="size-5" />
    </button>
  </div>
</header>
