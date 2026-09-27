<script lang="ts">
  import { startDrag } from '@crabnebula/tauri-plugin-drag';
  import Icon from './Icon.svelte';
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
  // Every library refresh hands over new entry objects; only a new path
  // should reload the thumbnail (otherwise all cards blink).
  let path = $derived(entry.path);
  let fav = $derived(!!entry.favorite);
  // The star bounces when it's switched on by a click (not when it just shows up).
  let bounce = $state(false);
  function toggleFavorite() {
    bounce = !fav;
    app.setFavorite(entry, !fav);
  }

  $effect(() => {
    const p = path;
    thumb = null;
    loaded = false;
    const io = new IntersectionObserver(
      async ([e]) => {
        if (!e.isIntersecting) return;
        io.disconnect();
        try {
          thumbPath = await api.thumbnail(p);
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

<!-- The star is a sibling of the card's button (buttons can't nest); it sits
     over the thumbnail's top-left corner. -->
<div class="card">
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
  <button
    class="star"
    class:on={fav}
    class:bounce={bounce && fav}
    aria-pressed={fav}
    aria-label={app.t(fav ? 'fav.remove' : 'fav.add')}
    title={app.t(fav ? 'fav.remove' : 'fav.add')}
    onclick={toggleFavorite}
  >
    <Icon name={fav ? 'starFill' : 'star'} size={15} stroke={1.8} />
  </button>
</div>

<style>
  .card {
    position: relative;
    min-width: 0;
  }
  .m {
    width: 100%;
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
  .card:hover .thumb,
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
  .card:hover img.loaded {
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
  /* Top-left belongs to the star. */
  .tag {
    right: 6px;
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
  /* Shown on hover (or keyboard focus); always shown, in gold, once starred. */
  .star {
    position: absolute;
    left: 6px;
    top: 6px;
    width: 26px;
    height: 26px;
    display: grid;
    place-items: center;
    border-radius: var(--r-sm);
    background: rgba(0, 0, 0, 0.6);
    color: #ececef;
    opacity: 0;
    transform: scale(0.85);
    transition:
      opacity var(--dur-fast) var(--ease),
      transform var(--dur-fast) var(--ease),
      background var(--dur-fast) var(--ease),
      color var(--dur-fast) var(--ease);
  }
  .card:hover .star,
  .star:focus-visible,
  .star.on {
    opacity: 1;
    transform: none;
  }
  .star:hover {
    background: rgba(0, 0, 0, 0.82);
  }
  .star.on {
    color: var(--fav);
  }
  .star.bounce {
    animation: star-pop 0.4s var(--ease);
  }
  @keyframes star-pop {
    40% {
      transform: scale(1.25);
    }
  }
</style>
