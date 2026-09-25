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

<div class="aurora" aria-hidden="true">
  <div class="blob a"></div>
  <div class="blob b"></div>
  <div class="blob c"></div>
</div>

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
  .aurora {
    position: fixed;
    inset: 0;
    overflow: hidden;
    pointer-events: none;
    z-index: 0;
  }
  .blob {
    position: absolute;
    border-radius: 50%;
    filter: blur(90px);
  }
  .a {
    width: 620px;
    height: 460px;
    left: -140px;
    top: -220px;
    background: color-mix(in srgb, var(--accent-a) 24%, transparent);
  }
  .b {
    width: 720px;
    height: 520px;
    right: -260px;
    bottom: -300px;
    background: color-mix(in srgb, var(--accent-b) 26%, transparent);
  }
  .c {
    width: 420px;
    height: 300px;
    right: 18%;
    top: -200px;
    background: rgba(56, 189, 248, 0.08);
  }
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
    padding: 4px 40px 40px;
  }
</style>
