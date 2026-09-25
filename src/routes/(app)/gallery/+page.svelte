<script lang="ts">
  import Icon from '$lib/components/Icon.svelte';
  import Select from '$lib/components/Select.svelte';
  import MediaCard from '$lib/components/MediaCard.svelte';
  import Viewer from '$lib/components/Viewer.svelte';
  import { api } from '$lib/api';
  import { app } from '$lib/app.svelte';
  import type { MediaEntry, MediaKind } from '$lib/types';

  let kind = $state<'all' | MediaKind>('all');
  let game = $state('');
  let query = $state('');
  let viewing = $state<MediaEntry | null>(null);

  let games = $derived([...new Set(app.media.map((m) => m.game).filter(Boolean))].sort((a, b) => a.localeCompare(b)));
  const count = (k: 'all' | MediaKind) => (k === 'all' ? app.media.length : app.media.filter((m) => m.kind === k).length);
  const tabs = [
    { value: 'all', key: 'gallery.all' },
    { value: 'clip', key: 'gallery.clips' },
    { value: 'recording', key: 'gallery.recordings' },
    { value: 'screenshot', key: 'gallery.screenshots' },
  ] as const;
  let list = $derived(
    app.media.filter(
      (m) =>
        (kind === 'all' || m.kind === kind) &&
        (!game || m.game === game) &&
        (!query || (m.name + ' ' + m.game).toLowerCase().includes(query.toLowerCase())),
    ),
  );
</script>

<div class="page">
  <div class="top">
    <nav class="tabs">
      {#each tabs as tab}
        <button class:active={kind === tab.value} onclick={() => (kind = tab.value)}>
          {app.t(tab.key)}<span class="n mono">{count(tab.value)}</span>
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
      {#each list as m (m.path)}
        <MediaCard entry={m} onopen={(e) => (viewing = e)} />
      {/each}
    </div>
  {:else if app.mediaLoaded}
    <p class="muted empty">{app.t('gallery.empty')}</p>
  {/if}
</div>

{#if viewing}
  <Viewer entry={viewing} {list} onclose={() => (viewing = null)} onselect={(e) => (viewing = e)} />
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
  .tabs button.active::after {
    content: '';
    position: absolute;
    left: 0;
    right: 0;
    bottom: -15px;
    height: 2px;
    background: var(--accent);
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
    width: 220px;
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
  .empty {
    padding: 40px 0;
    text-align: center;
  }
</style>
