<script lang="ts">
  import { onMount } from 'svelte';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import Sidebar from '$lib/components/Sidebar.svelte';
  import TitleBar from '$lib/components/TitleBar.svelte';
  import Notices from '$lib/components/Notices.svelte';
  import { app } from '$lib/app.svelte';
  import { page } from '$app/state';
  import { goto } from '$app/navigation';
  import { listen } from '@tauri-apps/api/event';
  import { rise, leave } from '$lib/motion';

  // Language-specific glyph shapes (Han characters in ja vs zh) and fonts
  // (see app.css) follow the <html lang>.
  $effect(() => {
    if (app.snapshot) document.documentElement.lang = app.lang;
  });

  let { children } = $props();
  let ready = $state(false);

  onMount(async () => {
    if (import.meta.env.DEV && !('__TAURI_INTERNALS__' in window)) {
      (await import('$lib/mock')).installMock();
    }
    try {
      await app.init();
      // The in-game menu can send the main window to a page.
      await listen<string>('app://navigate', (e) => goto(e.payload));
    } finally {
      ready = true;
      // Show only after the first render (no white flash).
      requestAnimationFrame(() => getCurrentWindow().show());
    }
  });
</script>

<div class="shell">
  <Sidebar />
  <main>
    <TitleBar />
    <div class="content">
      {#if ready}
        <!-- Old and new page share one grid cell, so they cross-fade in place. -->
        {#key page.url.pathname}
          <div class="view" in:rise={{ y: 8, delay: 60 }} out:leave>
            {@render children()}
          </div>
        {/key}
      {/if}
    </div>
  </main>
</div>
<Notices />

<style>
  .shell {
    position: relative;
    z-index: 1;
    display: flex;
    height: 100vh;
  }
  main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .content {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 0 32px 32px;
    display: grid;
    grid-template-rows: minmax(0, 1fr);
  }
  .view {
    grid-area: 1 / 1;
    min-width: 0;
    min-height: 0;
  }
</style>
