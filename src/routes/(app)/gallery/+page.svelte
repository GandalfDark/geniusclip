<script lang="ts">
  import IconSearch from '@tabler/icons-svelte-runes/icons/search';
  import IconFolder from '@tabler/icons-svelte-runes/icons/folder';
  import IconMovie from '@tabler/icons-svelte-runes/icons/movie';
  import Segmented from '$lib/components/Segmented.svelte';
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
  let count = (k: 'all' | MediaKind) => (k === 'all' ? app.media.length : app.media.filter((m) => m.kind === k).length);
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
    <h1>{app.t('gallery.title')}</h1>
    <div class="search">
      <IconSearch size={16} />
      <input placeholder={app.t('gallery.search')} bind:value={query} />
    </div>
    <button class="btn icon" title={app.t('set.open')} onclick={() => api.openMediaDir(kind === 'screenshot')}><IconFolder size={18} /></button>
  </div>

  <div class="filters">
    <Segmented
      value={kind}
      onchange={(v) => (kind = v)}
      options={[
        { value: 'all', label: `${app.t('gallery.all')} ${count('all')}` },
        { value: 'clip', label: `${app.t('gallery.clips')} ${count('clip')}` },
        { value: 'recording', label: `${app.t('gallery.recordings')} ${count('recording')}` },
        { value: 'screenshot', label: `${app.t('gallery.screenshots')} ${count('screenshot')}` },
      ]}
    />
    {#if games.length > 1}
      <Select
        width="220px"
        value={game}
        onchange={(v) => (game = v)}
        options={[{ value: '', label: app.t('gallery.allGames') }, ...games.map((g) => ({ value: g, label: g }))]}
      />
    {/if}
  </div>

  {#if list.length}
    <div class="grid">
      {#each list as m (m.path)}
        <MediaCard entry={m} onopen={(e) => (viewing = e)} />
      {/each}
    </div>
  {:else if app.mediaLoaded}
    <div class="empty">
      <div class="empty-icon"><IconMovie size={30} /></div>
      <div class="empty-title">{app.t('gallery.empty')}</div>
      <div class="muted">{app.t('gallery.emptyHint')}</div>
    </div>
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
    display: flex;
    align-items: center;
    gap: 14px;
    margin-bottom: 18px;
  }
  h1 {
    font-size: 26px;
    flex: 1;
  }
  .search {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 38px;
    width: 280px;
    padding: 0 12px;
    border-radius: var(--r);
    background: rgba(13, 10, 23, 0.6);
    border: 1px solid var(--line-2);
    color: var(--text-2);
  }
  .search:focus-within {
    border-color: var(--accent-a);
  }
  .search input {
    flex: 1;
    border: none;
    outline: none;
    background: none;
    user-select: text;
  }
  .filters {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-bottom: 20px;
    flex-wrap: wrap;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(250px, 1fr));
    gap: 16px;
  }
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding: 80px 0;
    text-align: center;
  }
  .empty-icon {
    width: 68px;
    height: 68px;
    border-radius: 22px;
    display: grid;
    place-items: center;
    background: var(--accent-soft);
    color: var(--accent-a);
    margin-bottom: 8px;
  }
  .empty-title {
    font-family: var(--font-display);
    font-size: 17px;
    font-weight: 600;
  }
</style>
