<script lang="ts">
  import { onMount } from 'svelte';
  import DropZone from './components/DropZone.svelte';
  import Header from './components/Header.svelte';
  import Metrics from './components/Metrics.svelte';
  import SettingsPanel from './components/SettingsPanel.svelte';
  import Toasts from './components/Toasts.svelte';
  import TorrentTable from './components/TorrentTable.svelte';
  import Icon from './components/Icon.svelte';
  import { app } from './lib/store.svelte';

  onMount(() => app.connect());
</script>

<main class="mx-auto flex max-w-6xl flex-col gap-5 px-4 py-6 sm:px-6 lg:py-10">
  <Header />
  {#if app.snapshot?.error}
    <div role="alert" class="flex items-start gap-3 rounded-2xl border border-rose-200 bg-rose-50 px-4 py-3 text-sm text-rose-900 dark:border-rose-900 dark:bg-rose-950 dark:text-rose-100">
      <Icon name="alert" class="mt-0.5 size-4 shrink-0" />
      <p>{app.snapshot.error}</p>
    </div>
  {/if}
  <Metrics />
  <DropZone />
  <TorrentTable />
  <footer class="pt-2 text-center text-xs text-zinc-400">ratiod · émulateur de ratio BitTorrent</footer>
</main>
<SettingsPanel />
<Toasts />
