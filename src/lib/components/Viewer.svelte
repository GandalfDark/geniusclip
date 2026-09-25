<script lang="ts">
  import { fade, scale } from 'svelte/transition';
  import IconX from '@tabler/icons-svelte-runes/icons/x';
  import IconFolder from '@tabler/icons-svelte-runes/icons/folder';
  import IconExternalLink from '@tabler/icons-svelte-runes/icons/external-link';
  import IconTrash from '@tabler/icons-svelte-runes/icons/trash';
  import IconPencil from '@tabler/icons-svelte-runes/icons/pencil';
  import IconScissors from '@tabler/icons-svelte-runes/icons/scissors';
  import IconArrowLeft from '@tabler/icons-svelte-runes/icons/arrow-left';
  import IconArrowRight from '@tabler/icons-svelte-runes/icons/arrow-right';
  import { api, fileUrl } from '$lib/api';
  import { app } from '$lib/app.svelte';
  import { bytes, date, preciseTime } from '$lib/format';
  import type { MediaEntry } from '$lib/types';

  let { entry, list, onclose, onselect }: { entry: MediaEntry; list: MediaEntry[]; onclose: () => void; onselect: (e: MediaEntry) => void } =
    $props();

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
    start = 0;
    end = 0;
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
    if (e.key === 'Escape') onclose();
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
      if (replace) onclose();
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
      next ? onselect(next) : onclose();
    } catch (e) {
      app.notify(String(e), 'error');
    }
  }
</script>

<svelte:window onkeydown={onkey} />

<div class="backdrop" transition:fade={{ duration: 150 }} onclick={onclose} role="presentation"></div>
<div class="viewer" transition:scale={{ start: 0.97, duration: 200 }}>
  <header>
    <div class="title">
      {#if renaming}
        <!-- svelte-ignore a11y_autofocus -->
        <input class="rename" bind:value={newName} autofocus onkeydown={(e) => e.key === 'Enter' && rename()} onblur={rename} />
      {:else}
        <h3>{entry.name}</h3>
      {/if}
      <div class="sub">
        {#if entry.game}<span class="chip">{entry.game}</span>{/if}
        <span class="muted">{date(entry.modified, app.lang)} · {bytes(entry.size, app.lang)}{entry.width ? ` · ${entry.width}×${entry.height}` : ''}</span>
      </div>
    </div>
    <div class="actions">
      <button class="btn sm ghost icon" title={app.t('gallery.rename')} onclick={() => ((newName = entry.name), (renaming = true))}><IconPencil size={17} /></button>
      <button class="btn sm ghost icon" title={app.t('gallery.reveal')} onclick={() => api.revealPath(entry.path)}><IconFolder size={17} /></button>
      <button class="btn sm ghost icon" title={app.t('gallery.open')} onclick={() => api.openPath(entry.path)}><IconExternalLink size={17} /></button>
      <button class="btn sm ghost danger" class:icon={!confirmDelete} title={app.t('gallery.delete')} onclick={del}>
        <IconTrash size={17} />{#if confirmDelete}<span>{app.t('gallery.delete')}?</span>{/if}
      </button>
      <span class="sep"></span>
      <button class="btn sm ghost icon" title={app.t('gallery.close')} onclick={onclose}><IconX size={19} /></button>
    </div>
  </header>

  <div class="stage">
    <button class="nav prev" disabled={index <= 0} onclick={() => go(-1)} aria-label="prev"><IconArrowLeft size={20} /></button>
    {#key entry.path}
      {#if isVideo}
        <!-- svelte-ignore a11y_media_has_caption -->
        <video bind:this={video} src={fileUrl(entry.path)} controls autoplay bind:currentTime={current} onloadedmetadata={loaded}></video>
      {:else}
        <img src={fileUrl(entry.path)} alt={entry.name} />
      {/if}
    {/key}
    <button class="nav next" disabled={index >= list.length - 1} onclick={() => go(1)} aria-label="next"><IconArrowRight size={20} /></button>
  </div>

  {#if isVideo && total > 0}
    <div class="trim" class:open={trimming}>
      <div class="trim-head">
        <button class="btn sm" class:active={trimming} onclick={() => (trimming = !trimming)}><IconScissors size={16} />{app.t('gallery.trim')}</button>
        {#if trimming}
          <span class="times">
            <span class="muted">{app.t('gallery.trimStart')}</span> <b>{preciseTime(start)}</b>
            <span class="muted">{app.t('gallery.trimEnd')}</span> <b>{preciseTime(end)}</b>
            <span class="len grad-text">{preciseTime(end - start)}</span>
          </span>
          <span class="grow"></span>
          <button class="btn sm ghost" onclick={setIn}>{app.t('gallery.setIn')}</button>
          <button class="btn sm ghost" onclick={setOut}>{app.t('gallery.setOut')}</button>
          <button class="btn sm" disabled={busy} onclick={() => trim(true)}>{app.t('gallery.replace')}</button>
          <button class="btn sm primary" disabled={busy} onclick={() => trim(false)}>{app.t('gallery.saveCopy')}</button>
        {/if}
      </div>
      {#if trimming}
        <div class="track" bind:this={track} onpointerdown={(e) => drag('seek', e)} role="presentation">
          <div class="sel" style:left="{(start / total) * 100}%" style:width="{((end - start) / total) * 100}%"></div>
          <div class="head" style:left="{(current / total) * 100}%"></div>
          <button class="handle" style:left="{(start / total) * 100}%" onpointerdown={(e) => (e.stopPropagation(), drag('start', e))} aria-label="start"></button>
          <button class="handle" style:left="{(end / total) * 100}%" onpointerdown={(e) => (e.stopPropagation(), drag('end', e))} aria-label="end"></button>
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgba(6, 4, 12, 0.72);
    backdrop-filter: blur(6px);
    z-index: 50;
  }
  .viewer {
    position: fixed;
    inset: 44px 36px 28px 112px;
    z-index: 51;
    display: flex;
    flex-direction: column;
    background: var(--panel-solid);
    border: 1px solid var(--line-2);
    border-radius: 22px;
    box-shadow: 0 40px 120px -30px rgba(0, 0, 0, 0.9);
    overflow: hidden;
  }
  header {
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 14px 16px 12px 22px;
    border-bottom: 1px solid var(--line);
  }
  .title {
    flex: 1;
    min-width: 0;
  }
  h3 {
    font-size: 16px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .rename {
    width: 100%;
    max-width: 520px;
    height: 32px;
    padding: 0 10px;
    border-radius: 8px;
    border: 1px solid var(--accent-a);
    background: rgba(13, 10, 23, 0.7);
    font-weight: 700;
    user-select: text;
  }
  .sub {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-top: 4px;
    font-size: 12.5px;
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .sep {
    width: 1px;
    height: 22px;
    background: var(--line-2);
    margin: 0 6px;
  }
  .stage {
    position: relative;
    flex: 1;
    min-height: 0;
    display: grid;
    place-items: center;
    background: #07050d;
  }
  video,
  img {
    max-width: 100%;
    max-height: 100%;
    width: 100%;
    height: 100%;
    object-fit: contain;
    outline: none;
  }
  .nav {
    position: absolute;
    top: 50%;
    margin-top: -22px;
    width: 44px;
    height: 44px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    background: rgba(26, 19, 48, 0.8);
    border: 1px solid var(--line-2);
    z-index: 2;
    opacity: 0;
    transition: opacity 0.2s;
  }
  .stage:hover .nav:not(:disabled) {
    opacity: 1;
  }
  .nav:disabled {
    display: none;
  }
  .prev {
    left: 16px;
  }
  .next {
    right: 16px;
  }
  .trim {
    padding: 12px 18px 14px;
    border-top: 1px solid var(--line);
  }
  .trim-head {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .trim-head .active {
    border-color: var(--accent-a);
  }
  .times {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
  }
  .times b {
    font-family: var(--font-display);
    font-weight: 500;
    margin-right: 8px;
  }
  .len {
    font-family: var(--font-display);
    font-weight: 600;
  }
  .grow {
    flex: 1;
  }
  .track {
    position: relative;
    height: 44px;
    margin-top: 12px;
    border-radius: 10px;
    background: repeating-linear-gradient(90deg, rgba(167, 139, 250, 0.1) 0 2px, transparent 2px 10px), rgba(13, 10, 23, 0.8);
    border: 1px solid var(--line);
    cursor: pointer;
  }
  .sel {
    position: absolute;
    top: -1px;
    bottom: -1px;
    border: 2px solid var(--accent-a);
    border-radius: 10px;
    background: color-mix(in srgb, var(--accent-a) 14%, transparent);
    pointer-events: none;
  }
  .head {
    position: absolute;
    top: -4px;
    bottom: -4px;
    width: 2px;
    margin-left: -1px;
    background: #fff;
    border-radius: 2px;
    pointer-events: none;
  }
  .handle {
    position: absolute;
    top: 50%;
    width: 14px;
    height: 34px;
    margin: -17px 0 0 -7px;
    border-radius: 6px;
    background: var(--accent-grad);
    box-shadow: 0 2px 10px rgba(0, 0, 0, 0.5);
    cursor: ew-resize;
  }
</style>
