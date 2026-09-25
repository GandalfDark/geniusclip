<script lang="ts">
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import Icon from './Icon.svelte';
  import { api } from '$lib/api';
  import { app } from '$lib/app.svelte';

  // Resolved lazily: in the browser preview the Tauri mock is installed after mount.
  const win = () => getCurrentWindow();
</script>

<header class="bar" data-tauri-drag-region>
  <div class="spacer" data-tauri-drag-region></div>
  <div class="controls">
    <button title={app.t('win.minimize')} onclick={() => win().minimize()}><Icon name="minimize" size={16} /></button>
    <button title={app.t('win.maximize')} onclick={() => win().toggleMaximize()}><Icon name="maximize" size={15} /></button>
    <button class="close" title={app.t('win.close')} onclick={() => api.closeMain()}><Icon name="close" size={16} /></button>
  </div>
</header>

<style>
  .bar {
    height: 36px;
    display: flex;
    flex-shrink: 0;
  }
  .spacer {
    flex: 1;
  }
  .controls {
    display: flex;
  }
  .controls button {
    width: 44px;
    display: grid;
    place-items: center;
    color: var(--text-3);
    transition:
      background 0.1s,
      color 0.1s;
  }
  .controls button:hover {
    background: var(--hover);
    color: var(--text);
  }
  .controls .close:hover {
    background: #c42b1c;
    color: #fff;
  }
</style>
