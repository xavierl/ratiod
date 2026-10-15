<script lang="ts">
  import { uploadTorrents } from '../lib/api';
  import { i18n, t } from '../lib/i18n/index.svelte';
  import { app } from '../lib/store.svelte';
  import { summarizeUploads } from '../lib/uploads';
  import Icon from './Icon.svelte';

  let dragDepth = $state(0);
  let busy = $state(false);
  let input: HTMLInputElement | undefined = $state();

  const hasFiles = (event: DragEvent) => event.dataTransfer?.types.includes('Files') ?? false;

  async function send(list: FileList | null | undefined) {
    const files = Array.from(list ?? []).filter((f) => f.name.toLowerCase().endsWith('.torrent'));
    if (files.length === 0) {
      app.notify('warning', t('drop.none'));
      return;
    }
    busy = true;
    try {
      const summary = summarizeUploads(await uploadTorrents(files), i18n.locale);
      app.notify(summary.kind, summary.text);
    } catch {
      app.notify('error', t('drop.failed'));
    } finally {
      busy = false;
    }
  }
</script>

<svelte:window
  ondragenter={(e) => {
    if (hasFiles(e)) {
      e.preventDefault();
      dragDepth++;
    }
  }}
  ondragleave={(e) => {
    if (hasFiles(e)) dragDepth = Math.max(0, dragDepth - 1);
  }}
  ondragover={(e) => {
    if (hasFiles(e)) e.preventDefault();
  }}
  ondrop={(e) => {
    if (hasFiles(e)) {
      e.preventDefault();
      dragDepth = 0;
      send(e.dataTransfer?.files);
    }
  }}
/>

<div class="flex items-center justify-center gap-3 rounded-2xl border-2 border-dashed border-zinc-300 px-4 py-5 text-sm text-zinc-500 dark:border-zinc-700">
  <Icon name="upload" class="size-5 shrink-0" />
  {#if busy}
    <span>{t('drop.busy')}</span>
  {:else}
    <span>
      {t('drop.before')} <span class="font-medium text-zinc-700 dark:text-zinc-300">.torrent</span> {t('drop.after')}
      <button class="font-medium text-emerald-600 underline-offset-2 hover:underline dark:text-emerald-400" onclick={() => input?.click()}>
        {t('drop.browse')}
      </button>
    </span>
  {/if}
  <input
    bind:this={input}
    class="hidden"
    type="file"
    accept=".torrent"
    multiple
    onchange={(e) => {
      send(e.currentTarget.files);
      e.currentTarget.value = '';
    }}
  />
</div>

{#if dragDepth > 0}
  <div class="pointer-events-none fixed inset-0 z-50 grid place-items-center bg-emerald-950/60 backdrop-blur-sm">
    <div class="rounded-3xl border-2 border-dashed border-emerald-300 px-12 py-10 text-center text-white">
      <Icon name="upload" class="mx-auto mb-3 size-8" />
      <p class="text-lg font-semibold">{t('drop.overlayTitle')}</p>
      <p class="text-sm text-emerald-100">{t('drop.overlayHint')}</p>
    </div>
  </div>
{/if}
