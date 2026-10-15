<script lang="ts">
  import { formatBytes, formatSpeed } from '../lib/format';
  import { i18n, t } from '../lib/i18n/index.svelte';
  import { app } from '../lib/store.svelte';

  const s = $derived(app.snapshot);
  const items = $derived([
    { label: t('metrics.globalSpeed'), value: formatSpeed(s?.globalSpeed ?? 0, i18n.locale) },
    { label: t('metrics.sessionUploaded'), value: formatBytes(s?.sessionUploaded ?? 0, i18n.locale) },
    { label: t('metrics.active'), value: s ? `${s.torrents.length} / ${s.config.simultaneousSeed}` : '—' },
    { label: t('metrics.queued'), value: String(s?.queued ?? 0) },
  ]);
</script>

<section class="grid grid-cols-2 gap-3 lg:grid-cols-4" aria-label={t('metrics.label')}>
  {#each items as item (item.label)}
    <div class="card px-4 py-3">
      <p class="text-xs font-medium tracking-wide text-zinc-500 uppercase">{item.label}</p>
      <p class="mt-1 text-2xl font-semibold tabular-nums">{item.value}</p>
    </div>
  {/each}
</section>
