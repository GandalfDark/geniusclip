<script lang="ts">
  import { page } from '$app/state';
  import Icon, { type IconName } from './Icon.svelte';
  import Logo from './Logo.svelte';
  import { app } from '$lib/app.svelte';

  const items: { href: string; icon: IconName; key: 'nav.home' | 'nav.gallery' | 'nav.settings' }[] = [
    { href: '/', icon: 'replay', key: 'nav.home' },
    { href: '/gallery', icon: 'film', key: 'nav.gallery' },
    { href: '/settings', icon: 'sliders', key: 'nav.settings' },
  ];

  let path = $derived(page.url.pathname);
  let live = $derived(!!app.status?.replayEnabled && !!app.status?.running);
</script>

<nav class="side" data-tauri-drag-region>
  <a class="logo" href="/" aria-label="GeniusClip"><Logo size={26} /></a>
  <div class="items">
    {#each items as it}
      {@const active = it.href === '/' ? path === '/' : path.startsWith(it.href)}
      <a class="item" class:active href={it.href} aria-label={app.t(it.key)}>
        <Icon name={it.icon} size={21} />
        <span class="tip">{app.t(it.key)}</span>
      </a>
    {/each}
  </div>
  <div class="state" title={live ? app.t('home.on') : app.t('home.off')}>
    <span class="dot" class:live></span>
    {#if app.status?.recording}<span class="rec mono">REC</span>{/if}
  </div>
</nav>

<style>
  .side {
    width: 56px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: 14px 0 18px;
    background: var(--bg-side);
    border-right: 1px solid var(--line);
  }
  .logo {
    display: grid;
    place-items: center;
    width: 40px;
    height: 40px;
    margin-bottom: 18px;
  }
  .items {
    display: flex;
    flex-direction: column;
    gap: 4px;
    flex: 1;
  }
  .item {
    position: relative;
    width: 40px;
    height: 40px;
    border-radius: var(--r);
    display: grid;
    place-items: center;
    color: var(--text-3);
    transition:
      color 0.12s,
      background 0.12s;
  }
  .item:hover {
    color: var(--text);
  }
  .item.active {
    color: var(--accent);
    background: var(--hover);
  }
  .item.active::before {
    content: '';
    position: absolute;
    left: -8px;
    top: 10px;
    bottom: 10px;
    width: 2px;
    background: var(--accent);
  }
  .tip {
    position: absolute;
    left: 48px;
    padding: 4px 8px;
    border-radius: var(--r-sm);
    background: var(--panel-2);
    border: 1px solid var(--line-2);
    color: var(--text);
    font-size: 12px;
    font-weight: 500;
    white-space: nowrap;
    opacity: 0;
    pointer-events: none;
    transition: opacity 0.12s;
    z-index: 20;
  }
  .item:hover .tip {
    opacity: 1;
  }
  .state {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--text-3);
  }
  .dot.live {
    background: var(--rec);
    animation: blink 1.6s steps(1) infinite;
  }
  .rec {
    font-size: 9px;
    font-weight: 700;
    color: var(--rec);
  }
  @keyframes blink {
    50% {
      opacity: 0.35;
    }
  }
</style>
