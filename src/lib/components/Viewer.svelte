<script lang="ts" module>
  export type Origin = { rect: DOMRect; src: string | null };
</script>

<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { fade } from 'svelte/transition';
  import { DUR, EASE, reduced } from '$lib/motion';
  import Icon from './Icon.svelte';
  import { api, fileUrl } from '$lib/api';
  import { app } from '$lib/app.svelte';
  import { bytes, date, preciseTime } from '$lib/format';
  import type { MediaEntry } from '$lib/types';

  let {
    entry,
    list,
    onclose,
    onselect,
    origin = null,
  }: { entry: MediaEntry; list: MediaEntry[]; onclose: () => void; onselect: (e: MediaEntry) => void; origin?: Origin | null } = $props();

  let panel: HTMLElement;
  let stageEl: HTMLElement;
  let ghostEl: HTMLImageElement | null = $state(null);
  let ghostSrc = $state<string | null>(null);
  let closing = false;

  // --- Container transform: the thumbnail grows from its card into the player.
  const aspect = () => (entry.width && entry.height ? entry.width / entry.height : 16 / 9);
  function fit(stage: DOMRect, ar: number) {
    let w = stage.width;
    let h = w / ar;
    if (h > stage.height) {
      h = stage.height;
      w = h * ar;
    }
    return new DOMRect(stage.left + (stage.width - w) / 2, stage.top + (stage.height - h) / 2, w, h);
  }
  async function fly(card: DOMRect, player: DOMRect, src: string, toCard: boolean) {
    ghostSrc = src;
    await tick();
    const g = ghostEl;
    if (!g) return;
    Object.assign(g.style, { left: `${player.left}px`, top: `${player.top}px`, width: `${player.width}px`, height: `${player.height}px` });
    const atCard = {
      transform: `translate(${card.left - player.left}px, ${card.top - player.top}px) scale(${card.width / player.width}, ${card.height / player.height})`,
      borderRadius: '6px',
    };
    const atPlayer = { transform: 'none', borderRadius: '0px' };
    const frames = toCard ? [atPlayer, atCard] : [atCard, atPlayer];
    await g.animate(frames, { duration: DUR + 80, easing: EASE, fill: 'forwards' }).finished;
  }

  onMount(async () => {
    if (!origin || reduced()) return;
    panel.animate([{ opacity: 0, transform: 'scale(0.985)' }, { opacity: 1, transform: 'none' }], { duration: DUR, easing: EASE });
    if (origin.src) {
      await fly(origin.rect, fit(stageEl.getBoundingClientRect(), aspect()), origin.src, false);
      await ghostEl?.animate([{ opacity: 1 }, { opacity: 0 }], { duration: 180, fill: 'forwards' }).finished;
      ghostSrc = null;
    }
  });

  async function close() {
    if (closing) return;
    closing = true;
    const card = document.querySelector<HTMLElement>(`[data-path="${CSS.escape(entry.path)}"] .thumb`);
    const img = card?.querySelector('img');
    if (card && !reduced()) {
      video?.pause();
      const player = fit(stageEl.getBoundingClientRect(), aspect());
      panel.animate([{ opacity: 1 }, { opacity: 0, transform: 'scale(0.985)' }], { duration: DUR, easing: EASE, fill: 'forwards' });
      if (img?.src) await fly(card.getBoundingClientRect(), player, img.src, true);
    }
    onclose();
  }

  let video = $state<HTMLVideoElement | null>(null);
  let current = $state(0);
  let total = $state(0);
  let start = $state(0);
  let end = $state(0);
  let trimming = $state(false);
  let busy = $state(false);
  let renaming = $state(false);
  let newName = $state('');
  let confirmDelete = $state(false);
  let track: HTMLElement | null = $state(null);

  let isVideo = $derived(entry.kind !== 'screenshot');
  let index = $derived(list.findIndex((e) => e.path === entry.path));

  $effect(() => {
    entry.path;
    trimming = false;
    renaming = false;
    confirmDelete = false;
  });

  function loaded() {
    total = video?.duration ?? 0;
    start = 0;
    end = total;
  }

  function go(delta: number) {
    const next = list[index + delta];
    if (next) onselect(next);
  }

  function onkey(e: KeyboardEvent) {
    if (renaming) return;
    if (e.key === 'Escape') close();
    else if (e.key === 'ArrowLeft' && (!isVideo || e.ctrlKey)) go(-1);
    else if (e.key === 'ArrowRight' && (!isVideo || e.ctrlKey)) go(1);
    else if (isVideo && e.code === 'Space') {
      e.preventDefault();
      if (video) video.paused ? video.play() : video.pause();
    } else if (isVideo && e.code === 'KeyI') setIn();
    else if (isVideo && e.code === 'KeyO') setOut();
  }

  function setIn() {
    trimming = true;
    start = Math.min(current, end - 0.5);
  }
  function setOut() {
    trimming = true;
    end = Math.max(current, start + 0.5);
  }

  function drag(which: 'start' | 'end' | 'seek', e: PointerEvent) {
    if (!track) return;
    const rect = track.getBoundingClientRect();
    const move = (ev: PointerEvent) => {
      const t = Math.max(0, Math.min(1, (ev.clientX - rect.left) / rect.width)) * total;
      if (which === 'start') start = Math.min(t, end - 0.5);
      else if (which === 'end') end = Math.max(t, start + 0.5);
      if (video) video.currentTime = which === 'seek' ? t : which === 'start' ? start : end;
    };
    move(e);
    const up = () => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', up);
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up);
  }

  async function trim(replace: boolean) {
    busy = true;
    try {
      video?.pause();
      const res = await api.trimMedia(entry.path, start, end, replace);
      app.notify(app.t('gallery.trimmed'), 'ok');
      await app.refreshMedia();
      if (replace) close();
      else if (res) onselect(res);
    } catch (e) {
      app.notify(String(e), 'error');
    } finally {
      busy = false;
    }
  }

  async function rename() {
    if (!newName.trim() || newName === entry.name) {
      renaming = false;
      return;
    }
    try {
      const path = await api.renameMedia(entry.path, newName.trim());
      await app.refreshMedia();
      const e = app.media.find((m) => m.path === path);
      if (e) onselect(e);
    } catch (e) {
      app.notify(String(e), 'error');
    }
    renaming = false;
  }

  async function del() {
    if (!confirmDelete) {
      confirmDelete = true;
      setTimeout(() => (confirmDelete = false), 3000);
      return;
    }
    const next = list[index + 1] ?? list[index - 1];
    try {
      await api.deleteMedia(entry.path);
      await app.refreshMedia();
      next ? onselect(next) : close();
    } catch (e) {
      app.notify(String(e), 'error');
    }
  }
</script>

<svelte:window onkeydown={onkey} />

<div class="backdrop" in:fade={{ duration: DUR }} out:fade={{ duration: 160 }} onclick={close} role="presentation"></div>
<div class="viewer" bind:this={panel}>
  <header>
    <div class="title">
      {#if renaming}
        <!-- svelte-ignore a11y_autofocus -->
        <input class="rename" bind:value={newName} autofocus onkeydown={(e) => e.key === 'Enter' && rename()} onblur={rename} />
      {:else}
        <h3>{entry.name}</h3>
      {/if}
      <div class="meta mono">
        {entry.game ? `${entry.game} · ` : ''}{date(entry.modified, app.lang)} · {bytes(entry.size, app.lang)}{entry.width ? ` · ${entry.width}×${entry.height}` : ''}
      </div>
    </div>
    <div class="actions">
      <button class="btn ghost icon" title={app.t('gallery.rename')} onclick={() => ((newName = entry.name), (renaming = true))}><Icon name="rename" size={18} /></button>
      <button class="btn ghost icon" title={app.t('gallery.reveal')} onclick={() => api.revealPath(entry.path)}><Icon name="folder" size={18} /></button>
      <button class="btn ghost icon" title={app.t('gallery.open')} onclick={() => api.openPath(entry.path)}><Icon name="open" size={18} /></button>
      <button class="btn ghost danger" class:icon={!confirmDelete} title={app.t('gallery.delete')} onclick={del}>
        <Icon name="trash" size={18} />{#if confirmDelete}{app.t('gallery.confirmDelete')}{/if}
      </button>
      <span class="sep"></span>
      <button class="btn ghost icon" title={app.t('gallery.close')} onclick={close}><Icon name="close" size={18} /></button>
    </div>
  </header>

  <div class="stage" bind:this={stageEl}>
    {#if index > 0}
      <button class="nav prev" onclick={() => go(-1)} aria-label="prev"><Icon name="left" size={20} /></button>
    {/if}
    {#key entry.path}
      {#if isVideo}
        <!-- svelte-ignore a11y_media_has_caption -->
        <video bind:this={video} src={fileUrl(entry.path)} controls autoplay bind:currentTime={current} onloadedmetadata={loaded}></video>
      {:else}
        <img src={fileUrl(entry.path)} alt={entry.name} />
      {/if}
    {/key}
    {#if index < list.length - 1}
      <button class="nav next" onclick={() => go(1)} aria-label="next"><Icon name="right" size={20} /></button>
    {/if}
  </div>

  {#if isVideo && total > 0}
    <div class="trim">
      <div class="trim-head">
        <button class="btn sm" class:on={trimming} onclick={() => (trimming = !trimming)}><Icon name="trim" size={16} />{app.t('gallery.trim')}</button>
        {#if trimming}
          <span class="times mono">{preciseTime(start)} – {preciseTime(end)} <span class="len">{preciseTime(end - start)}</span></span>
          <span class="grow"></span>
          <button class="btn sm ghost" onclick={setIn}>{app.t('gallery.setIn')}</button>
          <button class="btn sm ghost" onclick={setOut}>{app.t('gallery.setOut')}</button>
          <button class="btn sm" disabled={busy} onclick={() => trim(true)}>{app.t('gallery.replace')}</button>
          <button class="btn sm primary" disabled={busy} onclick={() => trim(false)}>{app.t('gallery.saveCopy')}</button>
        {/if}
      </div>
      {#if trimming}
        <div class="track" bind:this={track} onpointerdown={(e) => drag('seek', e)} role="presentation">
          <div class="dim" style:left="0" style:width="{(start / total) * 100}%"></div>
          <div class="dim" style:left="{(end / total) * 100}%" style:right="0"></div>
          <div class="sel" style:left="{(start / total) * 100}%" style:width="{((end - start) / total) * 100}%"></div>
          <div class="head" style:left="{(current / total) * 100}%"></div>
          <button class="handle in" style:left="{(start / total) * 100}%" onpointerdown={(e) => (e.stopPropagation(), drag('start', e))} aria-label="start"></button>
          <button class="handle out" style:left="{(end / total) * 100}%" onpointerdown={(e) => (e.stopPropagation(), drag('end', e))} aria-label="end"></button>
        </div>
      {/if}
    </div>
  {/if}
</div>

{#if ghostSrc}
  <img class="ghost" bind:this={ghostEl} src={ghostSrc} alt="" />
{/if}

<style>
  .ghost {
    position: fixed;
    z-index: 60;
    object-fit: cover;
    transform-origin: 0 0;
    pointer-events: none;
    will-change: transform;
  }
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgba(8, 8, 10, 0.78);
    z-index: 50;
  }
  .viewer {
    position: fixed;
    inset: 40px 28px 24px 84px;
    z-index: 51;
    display: flex;
    flex-direction: column;
    background: var(--panel);
    border: 1px solid var(--line-2);
    border-radius: var(--r-lg);
    overflow: hidden;
  }
  header {
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 10px 10px 10px 18px;
    border-bottom: 1px solid var(--line);
  }
  .title {
    flex: 1;
    min-width: 0;
  }
  h3 {
    font-size: 14.5px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .rename {
    width: 100%;
    max-width: 520px;
    height: 28px;
    padding: 0 8px;
    border-radius: var(--r-sm);
    border: 1px solid var(--accent);
    background: var(--bg);
    font-weight: 600;
    user-select: text;
  }
  .meta {
    margin-top: 2px;
    font-size: 11px;
    color: var(--text-3);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .actions {
    display: flex;
    align-items: center;
  }
  .sep {
    width: 1px;
    height: 20px;
    background: var(--line-2);
    margin: 0 6px;
  }
  .stage {
    position: relative;
    flex: 1;
    min-height: 0;
    display: grid;
    place-items: center;
    background: #0b0b0d;
  }
  video,
  img {
    width: 100%;
    height: 100%;
    object-fit: contain;
    outline: none;
  }
  .nav {
    position: absolute;
    top: 50%;
    margin-top: -20px;
    width: 40px;
    height: 40px;
    border-radius: var(--r);
    display: grid;
    place-items: center;
    background: rgba(20, 20, 22, 0.85);
    border: 1px solid var(--line-2);
    color: var(--text-2);
    z-index: 2;
    opacity: 0;
    transition: opacity 0.15s;
  }
  .nav:hover {
    color: var(--text);
  }
  .stage:hover .nav {
    opacity: 1;
  }
  .prev {
    left: 12px;
  }
  .next {
    right: 12px;
  }
  .trim {
    padding: 10px 14px 12px;
    border-top: 1px solid var(--line);
  }
  .trim-head {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .btn.on {
    border-color: var(--accent);
    color: var(--accent);
  }
  .times {
    font-size: 12px;
    color: var(--text-2);
    margin-left: 6px;
  }
  .len {
    color: var(--accent);
    margin-left: 8px;
  }
  .grow {
    flex: 1;
  }
  .track {
    position: relative;
    height: 36px;
    margin-top: 10px;
    border-radius: var(--r-sm);
    background: repeating-linear-gradient(90deg, transparent 0 7px, #26262c 7px 8px), var(--bg);
    border: 1px solid var(--line-2);
    cursor: pointer;
  }
  .dim {
    position: absolute;
    top: 0;
    bottom: 0;
    background: rgba(0, 0, 0, 0.55);
    pointer-events: none;
  }
  .sel {
    position: absolute;
    top: -1px;
    bottom: -1px;
    border-top: 2px solid var(--accent);
    border-bottom: 2px solid var(--accent);
    pointer-events: none;
  }
  .head {
    position: absolute;
    top: -3px;
    bottom: -3px;
    width: 2px;
    margin-left: -1px;
    background: #fff;
    pointer-events: none;
  }
  .handle {
    position: absolute;
    top: -1px;
    bottom: -1px;
    width: 10px;
    background: var(--accent);
    cursor: ew-resize;
  }
  .handle.in {
    margin-left: -10px;
    border-radius: 3px 0 0 3px;
  }
  .handle.out {
    border-radius: 0 3px 3px 0;
  }
  .handle::after {
    content: '';
    position: absolute;
    left: 4px;
    top: 12px;
    bottom: 12px;
    width: 2px;
    background: var(--accent-ink);
    opacity: 0.6;
  }
</style>
