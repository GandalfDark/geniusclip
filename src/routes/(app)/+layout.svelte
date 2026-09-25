<script lang="ts">
  import { onMount } from 'svelte';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import Sidebar from '$lib/components/Sidebar.svelte';
  import TitleBar from '$lib/components/TitleBar.svelte';
  import Notices from '$lib/components/Notices.svelte';
  import { app } from '$lib/app.svelte';

  let { children } = $props();
  let ready = $state(false);

  onMount(async () => {
    if (import.meta.env.DEV && !('__TAURI_INTERNALS__' in window)) {
      (await import('$lib/mock')).installMock();
    }
    try {
      await app.init();
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
        {@render children()}
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
  }
</style>
