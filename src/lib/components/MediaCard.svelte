<script lang="ts">
  import IconPlayerPlay from '@tabler/icons-svelte-runes/icons/player-play';
  import IconPhoto from '@tabler/icons-svelte-runes/icons/photo';
  import IconPlayerRecord from '@tabler/icons-svelte-runes/icons/player-record';
  import { api, fileUrl } from '$lib/api';
  import { app } from '$lib/app.svelte';
  import { bytes, date, duration } from '$lib/format';
  import type { MediaEntry } from '$lib/types';

  let { entry, onopen }: { entry: MediaEntry; onopen: (e: MediaEntry) => void } = $props();
  let thumb = $state<string | null>(null);
  let failed = $state(false);
  let el: HTMLElement;

  $effect(() => {
    const path = entry.path;
    thumb = null;
    failed = false;
    const io = new IntersectionObserver(
      async ([e]) => {
        if (!e.isIntersecting) return;
        io.disconnect();
        try {
          thumb = fileUrl(await api.thumbnail(path));
        } catch {
          failed = true;
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
    {#if thumb}
      <img src={thumb} alt="" draggable="false" />
    {:else}
      <div class="ph" class:failed></div>
    {/if}
    <div class="shade"></div>
    {#if entry.kind === 'screenshot'}
      <span class="badge"><IconPhoto size={13} /></span>
    {:else}
      <span class="dur">{duration(entry.duration)}</span>
      {#if entry.kind === 'recording'}<span class="badge rec"><IconPlayerRecord size={13} /></span>{/if}
      <span class="play"><IconPlayerPlay size={22} /></span>
    {/if}
  </div>
  <div class="meta">
    <div class="game">{entry.game || entry.name}</div>
    <div class="sub">{date(entry.modified, app.lang)} · {bytes(entry.size, app.lang)}</div>
  </div>
</button>

<style>
  .m {
    display: flex;
    flex-direction: column;
    text-align: left;
    border-radius: 16px;
    padding: 8px;
    background: var(--panel);
    border: 1px solid var(--line);
    transition:
      transform 0.2s var(--ease),
      border-color 0.2s,
      background 0.2s;
    min-width: 0;
  }
  .m:hover {
    transform: translateY(-3px);
    border-color: var(--line-2);
    background: rgba(40, 30, 70, 0.7);
  }
  .thumb {
    position: relative;
    aspect-ratio: 16 / 9;
    border-radius: 11px;
    overflow: hidden;
    background: #150f25;
  }
  img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
    transition: transform 0.4s var(--ease);
  }
  .m:hover img {
    transform: scale(1.04);
  }
  .ph {
    position: absolute;
    inset: 0;
    background: linear-gradient(100deg, #150f25 30%, #211838 50%, #150f25 70%);
    background-size: 200% 100%;
    animation: shimmer 1.4s infinite linear;
  }
  .ph.failed {
    animation: none;
  }
  @keyframes shimmer {
    to {
      background-position: -200% 0;
    }
  }
  .shade {
    position: absolute;
    inset: 0;
    background: linear-gradient(180deg, transparent 55%, rgba(8, 5, 16, 0.75));
  }
  .dur {
    position: absolute;
    right: 8px;
    bottom: 8px;
    font-family: var(--font-display);
    font-size: 11px;
    font-weight: 500;
    padding: 2px 7px;
    border-radius: 6px;
    background: rgba(8, 5, 16, 0.72);
  }
  .badge {
    position: absolute;
    left: 8px;
    top: 8px;
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border-radius: 7px;
    background: rgba(8, 5, 16, 0.72);
  }
  .badge.rec {
    color: #ff4d6d;
  }
  .play {
    position: absolute;
    left: 50%;
    top: 50%;
    width: 48px;
    height: 48px;
    margin: -24px 0 0 -24px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    background: var(--accent-grad);
    color: var(--accent-ink);
    opacity: 0;
    transform: scale(0.8);
    transition:
      opacity 0.2s,
      transform 0.25s var(--ease);
  }
  .m:hover .play {
    opacity: 1;
    transform: none;
  }
  .meta {
    padding: 10px 6px 4px;
    min-width: 0;
  }
  .game {
    font-weight: 700;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .sub {
    color: var(--text-2);
    font-size: 12px;
    margin-top: 1px;
  }
</style>
