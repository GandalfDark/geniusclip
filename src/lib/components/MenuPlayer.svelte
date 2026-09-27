<script lang="ts" module>
  export type PlayerOrigin = { rect: DOMRect; src: string };
</script>

<script lang="ts">
  // Clip player of the in-game menu: the thumbnail grows from the recent list
  // into the middle of the screen, so a replay can be watched without
  // leaving the game. Trimming stays in the main window.
  import { onMount, tick } from 'svelte';
  import Icon from './Icon.svelte';
  import { api, fileUrl } from '$lib/api';
  import { app } from '$lib/app.svelte';
  import { ago, bytes } from '$lib/format';
  import { DUR, EASE, reduced } from '$lib/motion';
  import type { MediaEntry } from '$lib/types';

  let { entry, origin = null, onclose }: { entry: MediaEntry; origin?: PlayerOrigin | null; onclose: () => void } = $props();

  let card: HTMLElement;
  let frame: HTMLElement;
  let video = $state<HTMLVideoElement | null>(null);
  let ghost = $state<HTMLImageElement | null>(null);
  let ghostSrc = $state<string | null>(null);
  let flying = $state(false);
  let copied = $state(false);
  let confirmDelete = $state(false);
  let closing = false;

  const ratio = $derived(entry.width && entry.height ? `${entry.width} / ${entry.height}` : '16 / 9');

  async function fly(thumb: DOMRect, to: DOMRect, src: string, back: boolean) {
    ghostSrc = src;
    await tick();
    const g = ghost;
    if (!g) return;
    Object.assign(g.style, { left: `${to.left}px`, top: `${to.top}px`, width: `${to.width}px`, height: `${to.height}px` });
    const atThumb = {
      transform: `translate(${thumb.left - to.left}px, ${thumb.top - to.top}px) scale(${thumb.width / to.width}, ${thumb.height / to.height})`,
      borderRadius: '8px',
    };
    const atFrame = { transform: 'none', borderRadius: '10px' };
    await g.animate(back ? [atFrame, atThumb] : [atThumb, atFrame], { duration: DUR + 60, easing: EASE, fill: 'forwards' }).finished;
  }

  onMount(async () => {
    if (!origin || reduced()) {
      video?.focus();
      return;
    }
    flying = true;
    card.animate([{ opacity: 0 }, { opacity: 1 }], { duration: DUR, easing: EASE });
    await fly(origin.rect, frame.getBoundingClientRect(), origin.src, false);
    flying = false;
    await ghost?.animate([{ opacity: 1 }, { opacity: 0 }], { duration: 160, fill: 'forwards' }).finished;
    ghostSrc = null;
    video?.focus();
  });

  /** Plays the opening in reverse (back into the list) when the thumbnail is still there. */
  export async function close() {
    if (closing) return;
    closing = true;
    video?.pause();
    const thumb = document.querySelector<HTMLElement>(`[data-path="${CSS.escape(entry.path)}"] .thumb`);
    const src = thumb?.querySelector('img')?.src;
    if (thumb && src && !reduced()) {
      const to = frame.getBoundingClientRect();
      flying = true;
      card.animate([{ opacity: 1 }, { opacity: 0 }], { duration: DUR, easing: EASE, fill: 'forwards' });
      await fly(thumb.getBoundingClientRect(), to, src, true);
    }
    onclose();
  }

  // Space plays/pauses, never clicks a button: after a mouse click focus stays
  // on it, and Space would click it again (on "Delete? Sure" that deletes the
  // clip). Buttons still work with Enter.
  function onkey(e: KeyboardEvent) {
    if (e.code !== 'Space' || e.defaultPrevented || !video) return;
    const at = document.activeElement;
    if (at === video) return; // the video's own controls handle it
    if (at instanceof HTMLElement && at.matches('input, select, textarea, [role="slider"]')) return;
    e.preventDefault();
    video.focus({ preventScroll: true });
    video.paused ? video.play() : video.pause();
  }

  async function send() {
    try {
      await api.copyMedia([entry.path]);
      copied = true;
      setTimeout(() => (copied = false), 2500);
    } catch (e) {
      app.notify(String(e), 'error');
    }
  }

  async function remove() {
    if (!confirmDelete) {
      confirmDelete = true;
      setTimeout(() => (confirmDelete = false), 3000);
      return;
    }
    try {
      await api.deleteMedia(entry.path);
      await app.refreshMedia();
      onclose();
    } catch (e) {
      app.notify(String(e), 'error');
    }
  }
</script>

<svelte:window onkeydown={onkey} />

<div class="stage">
  <div class="card" bind:this={card}>
    <header>
      <div class="title">
        <b>{entry.game || entry.name}</b>
        <span>{ago(entry.modified, app.lang)} · {bytes(entry.size, app.lang)}</span>
      </div>
      <button class="x" onclick={close} title={app.t('menu.close')}><Icon name="close" size={18} /></button>
    </header>
    <div class="frame" bind:this={frame} style:aspect-ratio={ratio}>
      <!-- svelte-ignore a11y_media_has_caption -->
      <video
        bind:this={video}
        class:hidden={flying}
        src={fileUrl(entry.path)}
        controls
        autoplay
        controlslist="nofullscreen nodownload noremoteplayback"
        disablepictureinpicture
      ></video>
    </div>
    <div class="actions">
      <button class="btn primary" onclick={send}><Icon name={copied ? 'check' : 'send'} size={16} />{app.t('gallery.send')}</button>
      <button class="btn" onclick={() => api.revealPath(entry.path)}><Icon name="folder" size={16} />{app.t('gallery.reveal')}</button>
      <button class="btn" onclick={() => api.openInApp('gallery', entry.path)}><Icon name="trim" size={16} />{app.t('menu.trimInApp')}</button>
      <span class="grow"></span>
      <button class="btn" class:danger={confirmDelete} onclick={remove}><Icon name="trash" size={16} />{app.t(confirmDelete ? 'gallery.confirmDelete' : 'gallery.delete')}</button>
    </div>
    {#if copied}<p class="hint">{app.t('gallery.pasteHint')}</p>{/if}
  </div>
</div>

{#if ghostSrc}
  <img class="ghost" bind:this={ghost} src={ghostSrc} alt="" />
{/if}

<style>
  /* The part of the screen right of the menu panel. */
  .stage {
    position: absolute;
    top: 0;
    bottom: 0;
    left: 360px;
    right: 0;
    display: grid;
    place-items: center;
    padding: 48px 56px;
    pointer-events: none;
  }
  .card {
    pointer-events: auto;
    width: min(100%, 1280px);
    max-height: 100%;
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 14px;
    border-radius: 16px;
    background: rgba(18, 18, 21, 0.97);
    border: 1px solid var(--line-2);
    box-shadow: 0 30px 90px -20px rgba(0, 0, 0, 0.8);
  }
  header {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0 2px 0 4px;
  }
  .title {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: baseline;
    gap: 10px;
  }
  .title b {
    font-size: 15px;
    font-weight: 650;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .title span {
    font-size: 12.5px;
    color: var(--text-3);
    white-space: nowrap;
  }
  .x {
    display: grid;
    place-items: center;
    width: 32px;
    height: 32px;
    border: 0;
    border-radius: 9px;
    background: none;
    color: var(--text-2);
    cursor: pointer;
  }
  .x:hover {
    background: var(--hover);
    color: var(--text);
  }
  .frame {
    position: relative;
    width: 100%;
    max-height: calc(100vh - 230px);
    border-radius: 10px;
    overflow: hidden;
    background: #000;
  }
  video {
    display: block;
    width: 100%;
    height: 100%;
    object-fit: contain;
    outline: none;
  }
  video.hidden {
    opacity: 0;
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .actions .btn {
    display: inline-flex;
    align-items: center;
    gap: 7px;
  }
  .actions .danger {
    color: var(--danger);
  }
  .grow {
    flex: 1;
  }
  .hint {
    margin: -2px 4px 0;
    font-size: 12.5px;
    color: var(--text-2);
  }
  .ghost {
    position: fixed;
    z-index: 10;
    object-fit: cover;
    transform-origin: 0 0;
    pointer-events: none;
  }
</style>
