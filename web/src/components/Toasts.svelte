<script lang="ts">
  import { fly } from 'svelte/transition';
  import { app } from '../lib/store.svelte';
  import Icon from './Icon.svelte';

  const tones = {
    success: 'border-emerald-200 bg-emerald-50 text-emerald-900 dark:border-emerald-900 dark:bg-emerald-950 dark:text-emerald-100',
    warning: 'border-amber-200 bg-amber-50 text-amber-900 dark:border-amber-900 dark:bg-amber-950 dark:text-amber-100',
    error: 'border-rose-200 bg-rose-50 text-rose-900 dark:border-rose-900 dark:bg-rose-950 dark:text-rose-100',
  } as const;
  const icons = { success: 'check', warning: 'alert', error: 'alert' } as const;
</script>

<div class="pointer-events-none fixed inset-x-0 bottom-0 z-50 flex flex-col items-center gap-2 p-4 sm:items-end" aria-live="polite">
  {#each app.toasts as toast (toast.id)}
    <div
      class="pointer-events-auto flex w-full max-w-sm items-start gap-3 rounded-xl border px-4 py-3 text-sm shadow-lg {tones[toast.kind]}"
      role="status"
      transition:fly={{ y: 16, duration: 150 }}
    >
      <Icon name={icons[toast.kind]} class="mt-0.5 size-4 shrink-0" />
      <p class="flex-1">{toast.text}</p>
      <button class="opacity-60 hover:opacity-100" aria-label="Fermer la notification" onclick={() => app.dismiss(toast.id)}>
        <Icon name="x" />
      </button>
    </div>
  {/each}
</div>
