<script lang="ts">
  import { formatBytes, formatSpeed } from '../lib/format';
  import { app } from '../lib/store.svelte';

  const s = $derived(app.snapshot);
  const items = $derived([
    { label: 'Débit global', value: formatSpeed(s?.globalSpeed ?? 0) },
    { label: 'Envoyé (session)', value: formatBytes(s?.sessionUploaded ?? 0) },
    { label: 'Actifs', value: s ? `${s.torrents.length} / ${s.config.simultaneousSeed}` : '—' },
    { label: 'En file', value: String(s?.queued ?? 0) },
  ]);
</script>

<section class="grid grid-cols-2 gap-3 lg:grid-cols-4" aria-label="Statistiques">
  {#each items as item (item.label)}
    <div class="card px-4 py-3">
      <p class="text-xs font-medium tracking-wide text-zinc-500 uppercase">{item.label}</p>
      <p class="mt-1 text-2xl font-semibold tabular-nums">{item.value}</p>
    </div>
  {/each}
</section>
