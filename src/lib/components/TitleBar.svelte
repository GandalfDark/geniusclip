<script lang="ts">
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import IconMinus from '@tabler/icons-svelte-runes/icons/minus';
  import IconSquare from '@tabler/icons-svelte-runes/icons/square';
  import IconX from '@tabler/icons-svelte-runes/icons/x';
  import { api } from '$lib/api';
  import { app } from '$lib/app.svelte';

  // Resolved lazily: in the browser preview the Tauri mock is installed after mount.
  const win = () => getCurrentWindow();
</script>

<header class="bar" data-tauri-drag-region>
  <div class="spacer" data-tauri-drag-region></div>
  <div class="controls">
    <button title={app.t('win.minimize')} onclick={() => win().minimize()}><IconMinus size={16} /></button>
    <button title={app.t('win.maximize')} onclick={() => win().toggleMaximize()}><IconSquare size={13} /></button>
    <button class="close" title={app.t('win.close')} onclick={() => api.closeMain()}><IconX size={17} /></button>
  </div>
</header>

<style>
  .bar {
    height: 40px;
    display: flex;
    align-items: stretch;
    flex-shrink: 0;
  }
  .spacer {
    flex: 1;
  }
  .controls {
    display: flex;
  }
  .controls button {
    width: 46px;
    display: grid;
    place-items: center;
    color: var(--text-2);
    transition:
      background 0.12s,
      color 0.12s;
  }
  .controls button:hover {
    background: var(--hover);
    color: var(--text);
  }
  .controls .close:hover {
    background: #e5484d;
    color: white;
  }
</style>
