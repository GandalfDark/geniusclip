<script lang="ts">
  import { page } from '$app/state';
  import IconBolt from '@tabler/icons-svelte-runes/icons/bolt';
  import IconLayoutGrid from '@tabler/icons-svelte-runes/icons/layout-grid';
  import IconSettings from '@tabler/icons-svelte-runes/icons/settings';
  import Logo from './Logo.svelte';
  import { app } from '$lib/app.svelte';

  const items = [
    { href: '/', icon: IconBolt, key: 'nav.home' as const },
    { href: '/gallery', icon: IconLayoutGrid, key: 'nav.gallery' as const },
    { href: '/settings', icon: IconSettings, key: 'nav.settings' as const },
  ];

  let path = $derived(page.url.pathname);
  let replayOn = $derived(!!app.status?.replayEnabled && !!app.status?.running);
  let recording = $derived(!!app.status?.recording);
</script>

<nav class="side" data-tauri-drag-region>
  <a class="logo" href="/" aria-label="GeniusClip"><Logo size={34} /></a>
  <div class="items">
    {#each items as it}
      {@const active = it.href === '/' ? path === '/' : path.startsWith(it.href)}
      <a class="item" class:active href={it.href} aria-label={app.t(it.key)}>
        <it.icon size={22} stroke={1.8} />
        <span class="tip">{app.t(it.key)}</span>
      </a>
    {/each}
  </div>
  <div class="state" title={replayOn ? app.t('home.replayOn') : app.t('home.replayOff')}>
    {#if recording}
      <span class="dot rec"></span>
    {/if}
    <span class="dot" class:on={replayOn}></span>
  </div>
</nav>

<style>
  .side {
    width: 76px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: 18px 0 20px;
    border-right: 1px solid var(--line);
    background: rgba(13, 10, 23, 0.55);
  }
  .logo {
    display: grid;
    place-items: center;
    width: 48px;
    height: 48px;
    margin-bottom: 26px;
  }
  .items {
    display: flex;
    flex-direction: column;
    gap: 8px;
    flex: 1;
  }
  .item {
    position: relative;
    width: 48px;
    height: 48px;
    border-radius: 14px;
    display: grid;
    place-items: center;
    color: var(--text-3);
    transition:
      color 0.15s,
      background 0.15s;
  }
  .item:hover {
    color: var(--text);
    background: var(--hover);
  }
  .item.active {
    color: var(--text);
    background: var(--active);
  }
  .item.active::before {
    content: '';
    position: absolute;
    left: -14px;
    top: 12px;
    bottom: 12px;
    width: 4px;
    border-radius: 0 4px 4px 0;
    background: var(--accent-grad);
  }
  .tip {
    position: absolute;
    left: 60px;
    padding: 5px 10px;
    border-radius: 8px;
    background: var(--panel-solid);
    border: 1px solid var(--line-2);
    font-size: 12.5px;
    font-weight: 600;
    white-space: nowrap;
    opacity: 0;
    transform: translateX(-4px);
    pointer-events: none;
    transition:
      opacity 0.15s,
      transform 0.15s var(--ease);
    z-index: 20;
  }
  .item:hover .tip {
    opacity: 1;
    transform: none;
  }
  .state {
    display: flex;
    flex-direction: column;
    gap: 8px;
    align-items: center;
  }
  .dot {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: var(--text-3);
  }
  .dot.on {
    background: var(--accent-a);
    box-shadow: 0 0 0 4px color-mix(in srgb, var(--accent-a) 20%, transparent);
    animation: pulse 2.4s infinite;
  }
  .dot.rec {
    background: #ff4d6d;
    box-shadow: 0 0 0 4px rgba(255, 77, 109, 0.2);
    animation: pulse 1.2s infinite;
  }
  @keyframes pulse {
    50% {
      box-shadow: 0 0 0 7px transparent;
    }
  }
</style>
