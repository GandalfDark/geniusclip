<script lang="ts">
  import { startDrag } from '@crabnebula/tauri-plugin-drag';
  import { api, fileUrl } from '$lib/api';
  import { app } from '$lib/app.svelte';
  import { bytes, date, duration } from '$lib/format';
  import type { MediaEntry } from '$lib/types';

  let { entry, onopen }: { entry: MediaEntry; onopen: (e: MediaEntry, from: DOMRect, thumb: string | null) => void } = $props();
  let thumb = $state<string | null>(null);
  let thumbPath: string | null = null;
  let loaded = $state(false);
  let el: HTMLElement;
  let frame: HTMLElement;
  let fresh = $derived(!!app.fresh[entry.path]);

  $effect(() => {
    const path = entry.path;
    thumb = null;
    loaded = false;
    const io = new IntersectionObserver(
      async ([e]) => {
        if (!e.isIntersecting) return;
        io.disconnect();
        try {
          thumbPath = await api.thumbnail(path);
          thumb = fileUrl(thumbPath);
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

<!-- Drag the card into Telegram/Explorer to attach the file itself. -->
<button
  class="m"
  class:fresh
  bind:this={el}
  data-path={entry.path}
  draggable="true"
  ondragstart={(e) => {
    e.preventDefault();
    if (thumbPath) startDrag({ item: [entry.path], icon: thumbPath }).catch(() => {});
  }}
  onclick={() => onopen(entry, frame.getBoundingClientRect(), loaded ? thumb : null)}
>
  <div class="thumb" bind:this={frame}>
    {#if thumb}<img src={thumb} alt="" draggable="false" class:loaded onload={() => (loaded = true)} />{/if}
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
    transition: outline-color var(--dur-fast) var(--ease);
  }
  .m:hover .thumb,
  .m.fresh .thumb {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }
  .m.fresh .thumb {
    animation: fresh 2.2s var(--ease);
  }
  @keyframes fresh {
    0% {
      box-shadow: 0 0 0 0 color-mix(in srgb, var(--accent) 60%, transparent);
    }
    40% {
      box-shadow: 0 0 0 6px transparent;
    }
  }
  img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
    opacity: 0;
    transform: scale(1.04);
    transition:
      opacity var(--dur) var(--ease),
      transform 0.6s var(--ease);
  }
  img.loaded {
    opacity: 1;
    transform: none;
  }
  .m:hover img.loaded {
    transform: scale(1.03);
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
