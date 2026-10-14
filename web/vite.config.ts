/// <reference types="vitest/config" />
import { svelte } from '@sveltejs/vite-plugin-svelte';
import tailwindcss from '@tailwindcss/vite';
import { defineConfig } from 'vite';

export default defineConfig({
  plugins: [tailwindcss(), svelte()],
  server: { proxy: { '/api': 'http://127.0.0.1:5081' } },
  test: { include: ['src/**/*.test.ts'] },
});
