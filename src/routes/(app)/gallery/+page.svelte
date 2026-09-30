<script lang="ts">
  import { onMount } from 'svelte';
  import { listen } from '@tauri-apps/api/event';
  import Icon from '$lib/components/Icon.svelte';
  import Select from '$lib/components/Select.svelte';
  import MediaCard from '$lib/components/MediaCard.svelte';
  import Viewer from '$lib/components/Viewer.svelte';
  import { api } from '$lib/api';
  import { enter } from '$lib/enter';
  import { LONG_LIST, leaveItem, move } from '$lib/motion';
  import type { Origin } from '$lib/components/Viewer.svelte';
  import { app } from '$lib/app.svelte';
  import type { MediaEntry, MediaKind } from '$lib/types';

  /** Which files the grid shows: all, one kind, or the starred ones. */
  type Tab = 'all' | MediaKind | 'fav';
  let kind = $state<Tab>('all');
  let game = $state('');
  let query = $state('');
  let viewing = $state<MediaEntry | null>(null);

  // A clip picked in the in-game menu ("Open in GeniusClip").
  async function openPending() {
    const path = await api.takePendingOpen().catch(() => null);
    if (!path) return;
    await app.refreshMedia();
    const e = app.media.find((m) => m.path === path);
    if (e) {
      origin = null;
      viewing = e;
    }
  }
  onMount(() => {
    openPending();
    const off = listen('app://navigate', openPending);
    return () => off.then((f) => f());
  });
  let origin = $state<Origin | null>(null);
  // The open clip can drop out of the filtered grid (unstarred on the
  // Favorites tab, renamed past the search): keep it in the viewer's list
  // where it was, so prev/next and delete still work from there.
  let lastIndex = 0;
  let viewerList = $derived.by(() => {
    if (!viewing) return list;
    const i = list.findIndex((e) => e.path === viewing!.path);
    if (i >= 0) {
      lastIndex = i;
      return list;
    }
    const at = Math.min(lastIndex, list.length);
    return [...list.slice(0, at), viewing, ...list.slice(at)];
  });
  function open(e: MediaEntry, rect: DOMRect, src: string | null) {
    origin = { rect, src };
    viewing = e;
  }

  let games = $derived([...new Set(app.media.map((m) => m.game).filter(Boolean))].sort((a, b) => a.localeCompare(b)));
  // The picked game's clips are all gone (or the picker is hidden with one
  // game left): drop the filter instead of showing an empty or stuck list.
  $effect(() => {
    if (game && app.mediaLoaded && (games.length < 2 || !games.includes(game))) game = '';
  });

  // Typing filters after a short pause, not on every key.
  let search = $state('');
  $effect(() => {
    const q = query.toLowerCase();
    if (!q) {
      search = '';
      return;
    }
    const t = setTimeout(() => (search = q), 150);
    return () => clearTimeout(t);
  });
  const inTab = (m: MediaEntry, k: Tab) => k === 'all' || (k === 'fav' ? m.favorite : m.kind === k);
  const count = (k: Tab) => (k === 'all' ? app.media.length : app.media.filter((m) => inTab(m, k)).length);
  const tabs = [
    { value: 'all', key: 'gallery.all' },
    { value: 'clip', key: 'gallery.clips' },
    { value: 'recording', key: 'gallery.recordings' },
    { value: 'screenshot', key: 'gallery.screenshots' },
    { value: 'fav', key: 'gallery.favorites' },
  ] as const;
  let list = $derived(
    app.media.filter(
      (m) =>
        inTab(m, kind) &&
        (!game || m.game === game) &&
        (!search || (m.name + ' ' + m.game).toLowerCase().includes(search)),
    ),
  );
  // Dozens of cards animating at once stutter: a long list just appears and
  // reflows (a freshly saved clip still pops in).
  let still = $derived(list.length > LONG_LIST);
</script>

<div class="page">
  <div class="top">
    <nav class="tabs">
      {#each tabs as tab}
        <button class:active={kind === tab.value} class:fav={tab.value === 'fav'} onclick={() => (kind = tab.value)}>
          {#if tab.value === 'fav'}<Icon name="starFill" size={13} />{/if}{app.t(tab.key)}<span class="n mono">{count(tab.value)}</span>
        </button>
      {/each}
    </nav>
    <span class="grow"></span>
    {#if games.length > 1}
      <Select
        width="190px"
        value={game}
        onchange={(v) => (game = v)}
        options={[{ value: '', label: app.t('gallery.allGames') }, ...games.map((g) => ({ value: g, label: g }))]}
      />
    {/if}
    <label class="search">
      <Icon name="search" size={16} />
      <input placeholder={app.t('gallery.search')} bind:value={query} />
    </label>
    <button class="btn icon" title={app.t('set.open')} onclick={() => api.openMediaDir(kind === 'screenshot')}><Icon name="folder" size={18} /></button>
  </div>

  {#if list.length}
    <div class="grid">
      {#each list as m, i (m.path)}
        <div class="cell" animate:move={{ still }} in:enter|global={{ i, fresh: !!app.fresh[m.path], still }} out:leaveItem>
          <MediaCard entry={m} onopen={open} />
        </div>
      {/each}
    </div>
  {:else if app.mediaLoaded}
    <p class="muted empty">{kind === 'fav' && !count('fav') ? app.t('gallery.favEmpty') : app.t('gallery.empty')}</p>
  {/if}
</div>

{#if viewing}
  <Viewer entry={viewing} list={viewerList} {origin} onclose={() => (viewing = null)} onselect={(e) => (viewing = e)} />
{/if}

<style>
  .page {
    max-width: 1400px;
    margin: 0 auto;
  }
  .top {
    position: sticky;
    top: 0;
    z-index: 5;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 0 14px;
    margin-bottom: 16px;
    background: var(--bg);
    border-bottom: 1px solid var(--line);
  }
  .tabs {
    display: flex;
    gap: 20px;
  }
  .tabs button {
    position: relative;
    height: 32px;
    white-space: nowrap;
    font-size: 14px;
    font-weight: 500;
    color: var(--text-3);
  }
  .tabs button:hover {
    color: var(--text);
  }
  .tabs button.active {
    color: var(--text);
  }
  .tabs button::after {
    content: '';
    position: absolute;
    left: 0;
    right: 0;
    bottom: -15px;
    height: 2px;
    background: var(--accent);
    transform: scaleX(0);
    transition: transform var(--dur) var(--ease);
  }
  .tabs button.active::after {
    transform: none;
  }
  .tabs button.fav {
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }
  .tabs button.fav.active :global(.ic) {
    color: var(--fav);
  }
  .tabs button.fav .n {
    margin-left: 1px;
  }
  .n {
    margin-left: 6px;
    font-size: 11px;
    color: var(--text-3);
  }
  .grow {
    flex: 1;
  }
  .search {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 32px;
    flex: 0 1 220px;
    min-width: 110px;
    padding: 0 9px;
    border-radius: var(--r);
    background: var(--bg);
    border: 1px solid var(--line-2);
    color: var(--text-3);
  }
  .search:focus-within {
    border-color: var(--accent);
  }
  .search input {
    flex: 1;
    min-width: 0;
    border: none;
    outline: none;
    background: none;
    font-size: 13px;
    user-select: text;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(230px, 1fr));
    gap: 20px 14px;
  }
  .cell {
    min-width: 0;
  }
  .empty {
    padding: 40px 0;
    text-align: center;
  }
</style>
