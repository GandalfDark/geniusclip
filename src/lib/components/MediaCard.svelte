<script lang="ts">
  import { api, fileUrl } from '$lib/api';
  import { app } from '$lib/app.svelte';
  import { bytes, date, duration } from '$lib/format';
  import type { MediaEntry } from '$lib/types';

  let { entry, onopen }: { entry: MediaEntry; onopen: (e: MediaEntry) => void } = $props();
  let thumb = $state<string | null>(null);
  let el: HTMLElement;

  $effect(() => {
    const path = entry.path;
    thumb = null;
    const io = new IntersectionObserver(
      async ([e]) => {
        if (!e.isIntersecting) return;
        io.disconnect();
        try {
          thumb = fileUrl(await api.thumbnail(path));
        } catch {
          /* keep the empty frame */
        }
      },
      { rootMargin: '300px' },
    );
    io.observe(el);
    return () => io.disconnect();
  });
</script>

<button class="m" bind:this={el} onclick={() => onopen(entry)}>
  <div class="thumb">
    {#if thumb}<img src={thumb} alt="" draggable="false" />{/if}
    {#if entry.kind === 'recording'}<span class="tag rec mono">REC</span>{/if}
    {#if entry.kind === 'screenshot'}
      <span class="tag mono">PNG</span>
    {:else}
      <span class="tc mono">{duration(entry.duration)}</span>
    {/if}
  </div>
  <div class="title">{entry.game || entry.name}</div>
  <div class="sub mono">{date(entry.modified, app.lang)} · {bytes(entry.size, app.lang)}</div>
</button>

<style>
  .m {
    display: flex;
    flex-direction: column;
    text-align: left;
    min-width: 0;
  }
  .thumb {
    position: relative;
    aspect-ratio: 16 / 9;
    border-radius: var(--r-sm);
    overflow: hidden;
    background: #1c1c21;
    outline: 1px solid var(--line);
    outline-offset: -1px;
    transition: outline-color 0.12s;
  }
  .m:hover .thumb {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }
  img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  .tc,
  .tag {
    position: absolute;
    font-size: 10.5px;
    font-weight: 500;
    padding: 1px 5px;
    border-radius: 2px;
    background: rgba(0, 0, 0, 0.78);
    color: #ececef;
  }
  .tc {
    right: 6px;
    bottom: 6px;
  }
  .tag {
    left: 6px;
    top: 6px;
  }
  .tag.rec {
    color: var(--rec);
  }
  .title {
    margin-top: 8px;
    font-size: 13px;
    font-weight: 500;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .sub {
    margin-top: 1px;
    font-size: 11px;
    color: var(--text-3);
  }
</style>
