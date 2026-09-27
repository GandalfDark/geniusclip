<script lang="ts" module>
  export type Origin = { rect: DOMRect; src: string | null };
</script>

<script lang="ts">
  import { onDestroy, onMount, tick } from 'svelte';
  import type { Attachment } from 'svelte/attachments';
  import { fade } from 'svelte/transition';
  import { DUR, EASE, reduced, rise } from '$lib/motion';
  import Icon, { type IconName } from './Icon.svelte';
  import Waveform from './Waveform.svelte';
  import { api, fileUrl, type ClipAudio } from '$lib/api';
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
  let alive = true;
  onDestroy(() => (alive = false));
  // Focus moves into the dialog; on close it goes back to the card of the
  // clip shown last, or to whatever had it before.
  const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;

  /** After an await: is the viewer still open on the clip the action began with?
   *  (Esc or another clip meanwhile must not be acted on.) */
  const still = (path: string) => alive && !closing && entry.path === path;

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
    panel.focus({ preventScroll: true });
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
    const cardEl = document.querySelector<HTMLElement>(`[data-path="${CSS.escape(entry.path)}"]`);
    const card = cardEl?.querySelector<HTMLElement>('.thumb');
    const img = card?.querySelector('img');
    if (card && !reduced()) {
      video?.pause();
      const player = fit(stageEl.getBoundingClientRect(), aspect());
      panel.animate([{ opacity: 1 }, { opacity: 0, transform: 'scale(0.985)' }], { duration: DUR, easing: EASE, fill: 'forwards' });
      if (img?.src) await fly(card.getBoundingClientRect(), player, img.src, true);
    }
    (cardEl ?? (opener?.isConnected ? opener : null))?.focus({ preventScroll: true });
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
  let copied = $state(false);

  async function send() {
    try {
      await api.copyMedia([entry.path]);
      copied = true;
      app.notify(app.t('gallery.pasteHint'), 'ok');
      setTimeout(() => (copied = false), 1800);
    } catch (e) {
      app.notify(String(e), 'error');
    }
  }
  let track: HTMLElement | null = $state(null);

  let isVideo = $derived(entry.kind !== 'screenshot');
  let index = $derived(list.findIndex((e) => e.path === entry.path));

  $effect(() => {
    entry.path;
    trimming = false;
    renaming = false;
    confirmDelete = false;
    audio = null;
    lanes = [];
    peaks = {};
    audioFor = '';
  });

  // --- Audio lanes: per-track volume, heard live before saving.
  type Lane = { track: number; label: string; icon: IconName; offIcon: IconName; file: string; gain: number; muted: boolean };
  let audio = $state.raw<ClipAudio | null>(null);
  let lanes = $state<Lane[]>([]);
  // Thousands of levels per track, never edited: kept out of the deeply
  // reactive lanes so they aren't wrapped in proxies.
  let peaks = $state.raw<Record<number, number[]>>({});
  let audioFor = '';

  async function loadAudio(path: string) {
    audioFor = path;
    try {
      const a = await api.clipAudio(path);
      if (entry.path !== path) return;
      // GeniusClip clips: show game and mic; the mix is rebuilt from them.
      const tracks = a.mix ? [1, 2] : a.titles.map((_, k) => k);
      audio = a;
      peaks = Object.fromEntries(tracks.map((k) => [k, a.peaks[k] ?? []]));
      lanes = tracks
        .filter((k) => a.files[k])
        .map((k, n) => {
          const game = a.mix && k === 1;
          const mic = a.mix && k === 2;
          const title = a.titles[k] && a.titles[k] !== 'SoundHandler' ? a.titles[k] : app.t('trim.track', { n: n + 1 });
          return {
            track: k,
            label: game ? app.t('trim.game') : mic ? app.t('trim.mic') : title,
            icon: game ? 'game' : mic ? 'mic' : 'speaker',
            offIcon: mic ? 'micOff' : 'speakerOff',
            file: a.files[k],
            gain: 1,
            muted: false,
          };
        });
    } catch (e) {
      console.warn('clip audio', e);
    }
  }

  $effect(() => {
    if (trimming && isVideo && audioFor !== entry.path) loadAudio(entry.path);
  });

  // While the lanes are shown, the video's own (mixed) sound is silenced in
  // the audio graph and the separate tracks play in sync through gain nodes,
  // so volume changes are heard immediately and can go above 100%.
  // The video stays connected (at zero gain): Chromium drives the video clock
  // from its audio, and an unconnected source would freeze playback.
  let actx: AudioContext | null = null;
  /** The video's own route into the graph (an element can be wired only once). */
  let videoNodes: { el: HTMLVideoElement; src: MediaElementAudioSourceNode; gain: GainNode } | null = null;
  let players = $state.raw<{ el: HTMLAudioElement; gain: GainNode; src: MediaElementAudioSourceNode }[]>([]);
  let previewing = $derived(trimming && lanes.length > 0 && !!video);

  // Each clip gets a new <video>: the previous one's nodes are let go, or its
  // decoder stays alive until the viewer closes.
  function releaseVideo(el?: HTMLVideoElement) {
    if (!videoNodes || (el && videoNodes.el !== el)) return;
    videoNodes.src.disconnect();
    videoNodes.gain.disconnect();
    videoNodes = null;
  }

  $effect(() => {
    const v = video;
    return () => {
      if (v) releaseVideo(v);
    };
  });

  $effect(() => {
    if (!previewing || !video) return;
    const v = video;
    const files = lanes.map((l) => l.file);
    const ctx = (actx ??= new AudioContext());
    if (videoNodes?.el !== v) {
      releaseVideo();
      const src = ctx.createMediaElementSource(v);
      const gain = ctx.createGain();
      src.connect(gain).connect(ctx.destination);
      videoNodes = { el: v, src, gain };
    }
    const vGain = videoNodes.gain;
    vGain.gain.value = 0;
    const master = ctx.createGain();
    master.connect(ctx.destination);
    const ps = files.map((f) => {
      const el = new Audio();
      el.crossOrigin = 'anonymous';
      el.preload = 'auto';
      el.src = fileUrl(f);
      const src = ctx.createMediaElementSource(el);
      const gain = ctx.createGain();
      src.connect(gain).connect(master);
      return { el, gain, src };
    });
    players = ps;

    const volume = () => master.gain.setTargetAtTime(v.muted ? 0 : v.volume, ctx.currentTime, 0.01);
    const sync = (force: boolean) => {
      for (const p of ps) {
        if (force || Math.abs(p.el.currentTime - v.currentTime) > 0.08) p.el.currentTime = v.currentTime;
        p.el.playbackRate = v.playbackRate;
      }
    };
    const play = () => {
      ctx.resume();
      sync(true);
      for (const p of ps) p.el.play().catch(() => {});
    };
    const pause = () => {
      for (const p of ps) p.el.pause();
      sync(true);
    };
    const stall = () => ps.forEach((p) => p.el.pause());
    const seeked = () => sync(true);
    const drift = () => !v.paused && sync(false);
    const events: [string, () => void][] = [
      ['playing', play],
      ['pause', pause],
      ['waiting', stall],
      ['seeked', seeked],
      ['ratechange', drift],
      ['timeupdate', drift],
      ['volumechange', volume],
    ];
    for (const [n, f] of events) v.addEventListener(n, f);
    volume();
    if (!v.paused) play();
    else sync(true);

    return () => {
      for (const [n, f] of events) v.removeEventListener(n, f);
      for (const p of ps) {
        p.el.pause();
        p.src.disconnect();
        p.gain.disconnect();
        p.el.removeAttribute('src');
        p.el.load();
      }
      master.disconnect();
      players = [];
      vGain.gain.value = 1;
    };
  });

  $effect(() => {
    const ps = players;
    const ctx = actx;
    if (!ctx) return;
    lanes.forEach((l, i) => ps[i]?.gain.gain.setTargetAtTime(l.muted ? 0 : l.gain, ctx.currentTime, 0.015));
  });

  $effect(() => () => {
    actx?.close();
  });

  const clampGain = (g: number) => Math.round(Math.max(0, Math.min(2, g)) * 100) / 100;

  function setGain(l: Lane, g: number) {
    l.gain = clampGain(g);
    if (l.gain > 0) l.muted = false;
  }

  // Vertical drag on a lane sets its volume; a click without moving seeks.
  function laneDrag(l: Lane, e: PointerEvent) {
    const el = e.currentTarget as HTMLElement;
    const rect = el.getBoundingClientRect();
    const y0 = e.clientY;
    const g0 = l.gain;
    let moved = false;
    el.setPointerCapture(e.pointerId);
    const move = (ev: PointerEvent) => {
      if (!moved && Math.abs(ev.clientY - y0) < 4) return;
      moved = true;
      const g = g0 + (y0 - ev.clientY) / 70;
      setGain(l, Math.abs(g - 1) < 0.04 ? 1 : g);
    };
    const up = (ev: PointerEvent) => {
      el.removeEventListener('pointermove', move);
      el.removeEventListener('pointerup', up);
      if (!moved && video) video.currentTime = Math.max(0, Math.min(1, (ev.clientX - rect.left) / rect.width)) * total;
    };
    el.addEventListener('pointermove', move);
    el.addEventListener('pointerup', up);
  }

  function laneKey(l: Lane, e: KeyboardEvent) {
    if (e.key !== 'ArrowUp' && e.key !== 'ArrowDown') return;
    e.preventDefault();
    setGain(l, l.gain + (e.key === 'ArrowUp' ? 0.05 : -0.05));
  }

  // Wheel needs a non-passive listener to keep the page from scrolling.
  const wheel =
    (l: Lane): Attachment<HTMLElement> =>
    (node) => {
      const on = (e: WheelEvent) => {
        e.preventDefault();
        setGain(l, Math.round((l.gain + (e.deltaY < 0 ? 0.05 : -0.05)) * 20) / 20);
      };
      node.addEventListener('wheel', on, { passive: false });
      return () => node.removeEventListener('wheel', on);
    };

  function trackGains(): number[] | null {
    if (!audio || !lanes.some((l) => l.muted || l.gain !== 1)) return null;
    return audio.titles.map((_, k) => {
      const l = lanes.find((x) => x.track === k);
      return l ? (l.muted ? 0 : l.gain) : 1;
    });
  }

  function loaded() {
    total = video?.duration ?? 0;
    start = 0;
    end = total;
  }

  function go(delta: number) {
    // A save, rename or delete in progress finishes on the clip it started on.
    if (busy) return;
    const next = list[index + delta];
    if (next) onselect(next);
  }

  // The viewer is modal: Tab cycles through its own controls only.
  const FOCUSABLE = 'button:not(:disabled), input:not(:disabled), video[controls], [tabindex]:not([tabindex="-1"])';
  function trapTab(e: KeyboardEvent) {
    const items = [...panel.querySelectorAll<HTMLElement>(FOCUSABLE)].filter((el) => el.getClientRects().length > 0);
    const at = document.activeElement;
    const inside = !!at && at !== panel && panel.contains(at);
    const edge = e.shiftKey ? items[0] : items[items.length - 1];
    if (inside && at !== edge) return;
    e.preventDefault();
    (e.shiftKey ? items[items.length - 1] : items[0])?.focus();
  }

  /** Space and the like belong to a focused control (button, field, slider). */
  function onControl() {
    const at = document.activeElement;
    return !!at && at !== panel && at.matches('button, input, select, textarea, a[href], [role], [tabindex]:not([tabindex="-1"])');
  }

  function onkey(e: KeyboardEvent) {
    if (closing) return;
    if (e.key === 'Tab') return trapTab(e);
    // Already handled by the focused control (a trim handle, a lane).
    if (renaming || e.defaultPrevented) return;
    if (e.key === 'Escape') close();
    else if (e.key === 'ArrowLeft' && (!isVideo || e.ctrlKey)) go(-1);
    else if (e.key === 'ArrowRight' && (!isVideo || e.ctrlKey)) go(1);
    else if (isVideo && e.code === 'Space' && !onControl()) {
      e.preventDefault();
      if (video) video.paused ? video.play() : video.pause();
    } else if (isVideo && e.code === 'KeyI') setIn();
    else if (isVideo && e.code === 'KeyO') setOut();
  }

  /** Shortest selection, in seconds. */
  const MIN_LEN = 0.5;
  const tenth = (t: number) => Math.round(t * 10) / 10;

  function setIn() {
    trimming = true;
    start = Math.min(current, end - MIN_LEN);
  }
  function setOut() {
    trimming = true;
    end = Math.max(current, start + MIN_LEN);
  }

  /** Moves a trim handle (kept inside the clip and apart from the other one)
   *  and shows that frame. */
  function setHandle(which: 'start' | 'end', t: number) {
    if (which === 'start') start = Math.max(0, Math.min(t, end - MIN_LEN));
    else end = Math.min(total, Math.max(t, start + MIN_LEN));
    if (video) video.currentTime = which === 'start' ? start : end;
  }

  // Arrows nudge a handle by 0.1 s (Shift: 1 s), PageUp/PageDown by 10 s,
  // Home/End take it as far as it goes.
  function handleKey(which: 'start' | 'end', e: KeyboardEvent) {
    const at = which === 'start' ? start : end;
    const step = e.shiftKey ? 1 : 0.1;
    const to: Record<string, number> = {
      ArrowLeft: at - step,
      ArrowDown: at - step,
      ArrowRight: at + step,
      ArrowUp: at + step,
      PageDown: at - 10,
      PageUp: at + 10,
      Home: 0,
      End: total,
    };
    if (!(e.key in to)) return;
    e.preventDefault();
    setHandle(which, tenth(to[e.key]));
  }

  function drag(which: 'start' | 'end' | 'seek', e: PointerEvent) {
    if (!track) return;
    const rect = track.getBoundingClientRect();
    const move = (ev: PointerEvent) => {
      const t = Math.max(0, Math.min(1, (ev.clientX - rect.left) / rect.width)) * total;
      if (which === 'seek') {
        if (video) video.currentTime = t;
      } else setHandle(which, t);
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
    if (busy) return;
    const path = entry.path;
    busy = true;
    try {
      video?.pause();
      const res = await api.trimMedia(path, start, end, replace, trackGains());
      app.notify(app.t('gallery.trimmed'), 'ok');
      await app.refreshMedia();
      if (!still(path)) return;
      if (replace) close();
      else if (res) onselect(res);
    } catch (e) {
      app.notify(String(e), 'error');
    } finally {
      busy = false;
    }
  }

  function startRename() {
    newName = entry.name;
    renaming = true;
  }

  // Enter and blur both commit, Esc cancels; whichever comes first ends the
  // editing, so the blur that follows (the field goes away) does nothing.
  async function rename() {
    if (!renaming) return;
    renaming = false;
    const name = newName.trim();
    const path = entry.path;
    if (!name || name === entry.name || busy) return;
    busy = true;
    try {
      const to = await api.renameMedia(path, name);
      await app.refreshMedia();
      if (!still(path)) return;
      const e = app.media.find((m) => m.path === to);
      if (e) onselect(e);
    } catch (e) {
      app.notify(String(e), 'error');
    } finally {
      busy = false;
    }
  }

  function renameKey(e: KeyboardEvent) {
    if (e.key === 'Enter') rename();
    else if (e.key === 'Escape') {
      // Only the field: the viewer itself stays open.
      e.stopPropagation();
      renaming = false;
    }
  }

  async function del() {
    if (busy) return;
    if (!confirmDelete) {
      confirmDelete = true;
      setTimeout(() => (confirmDelete = false), 3000);
      return;
    }
    const path = entry.path;
    const next = list[index + 1] ?? list[index - 1];
    busy = true;
    try {
      await api.deleteMedia(path);
      await app.refreshMedia();
      if (!still(path)) return;
      next ? onselect(next) : close();
    } catch (e) {
      app.notify(String(e), 'error');
    } finally {
      busy = false;
    }
  }
</script>

<svelte:window onkeydown={onkey} />

<div class="backdrop" in:fade={{ duration: DUR }} out:fade={{ duration: 160 }} onclick={close} role="presentation"></div>
<div class="viewer" bind:this={panel} role="dialog" aria-modal="true" aria-label={entry.name} tabindex="-1">
  <header>
    <div class="title">
      {#if renaming}
        <!-- svelte-ignore a11y_autofocus -->
        <input class="rename" bind:value={newName} autofocus onkeydown={renameKey} onblur={rename} />
      {:else}
        <h3>{entry.name}</h3>
      {/if}
      <div class="meta mono">
        {entry.game ? `${entry.game} · ` : ''}{date(entry.modified, app.lang)} · {bytes(entry.size, app.lang)}{entry.width ? ` · ${entry.width}×${entry.height}` : ''}
      </div>
    </div>
    <div class="actions">
      <button class="btn send" class:done={copied} title={app.t('gallery.sendHint')} onclick={send}>
        {#key copied}
          <span class="send-in" in:rise={{ y: 5, duration: 240 }}>
            {#if copied}<Icon name="check" size={17} stroke={2.2} />{app.t('gallery.copied')}{:else}<Icon name="send" size={17} />{app.t('gallery.send')}{/if}
          </span>
        {/key}
      </button>
      <span class="sep"></span>
      <button class="btn ghost icon" title={app.t('gallery.rename')} disabled={busy} onclick={startRename}><Icon name="rename" size={18} /></button>
      <button class="btn ghost icon" title={app.t('gallery.reveal')} onclick={() => api.revealPath(entry.path)}><Icon name="folder" size={18} /></button>
      <button class="btn ghost icon" title={app.t('gallery.open')} onclick={() => api.openPath(entry.path)}><Icon name="open" size={18} /></button>
      <button class="btn ghost danger" class:icon={!confirmDelete} title={app.t('gallery.delete')} disabled={busy} onclick={del}>
        <Icon name="trash" size={18} />{#if confirmDelete}{app.t('gallery.confirmDelete')}{/if}
      </button>
      <span class="sep"></span>
      <button class="btn ghost icon" title={app.t('gallery.close')} onclick={close}><Icon name="close" size={18} /></button>
    </div>
  </header>

  <div class="stage" bind:this={stageEl}>
    {#if index > 0}
      <button class="nav prev" disabled={busy} onclick={() => go(-1)} aria-label={app.t('gallery.prev')}><Icon name="left" size={20} /></button>
    {/if}
    {#key entry.path}
      {#if isVideo}
        <!-- svelte-ignore a11y_media_has_caption -->
        <video bind:this={video} src={fileUrl(entry.path)} crossorigin="anonymous" controls autoplay bind:currentTime={current} onloadedmetadata={loaded}></video>
      {:else}
        <img src={fileUrl(entry.path)} alt={entry.name} />
      {/if}
    {/key}
    {#if index < list.length - 1}
      <button class="nav next" disabled={busy} onclick={() => go(1)} aria-label={app.t('gallery.next')}><Icon name="right" size={20} /></button>
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
        <div class="timeline" in:rise={{ y: 6, duration: 240 }}>
          <div class="heads">
            <div class="lane-head video-head"><Icon name="film" size={16} />{app.t('trim.video')}</div>
            {#each lanes as l (l.track)}
              <div class="lane-head" in:fade={{ duration: 200 }}>
                <button class="mute" class:off={l.muted} title={app.t(l.muted ? 'trim.unmute' : 'trim.mute')} onclick={() => (l.muted = !l.muted)}>
                  <Icon name={l.muted ? l.offIcon : l.icon} size={17} />
                </button>
                <span class="name">{l.label}</span>
                <span class="val mono" class:changed={!l.muted && l.gain !== 1}>{l.muted ? '—' : `${Math.round(l.gain * 100)}%`}</span>
              </div>
            {/each}
          </div>
          <div class="bodies">
          <div class="track" bind:this={track} onpointerdown={(e) => drag('seek', e)} role="presentation">
            <div class="dim" style:left="0" style:width="{(start / total) * 100}%"></div>
            <div class="dim" style:left="{(end / total) * 100}%" style:right="0"></div>
            <div class="sel" style:left="{(start / total) * 100}%" style:width="{((end - start) / total) * 100}%"></div>
            <div
              class="handle in"
              style:left="{(start / total) * 100}%"
              role="slider"
              tabindex="0"
              aria-label={app.t('trim.start')}
              aria-valuemin={0}
              aria-valuemax={tenth(end - MIN_LEN)}
              aria-valuenow={tenth(start)}
              aria-valuetext={preciseTime(start)}
              onpointerdown={(e) => (e.stopPropagation(), drag('start', e))}
              onkeydown={(e) => handleKey('start', e)}
            ></div>
            <div
              class="handle out"
              style:left="{(end / total) * 100}%"
              role="slider"
              tabindex="0"
              aria-label={app.t('trim.end')}
              aria-valuemin={tenth(start + MIN_LEN)}
              aria-valuemax={tenth(total)}
              aria-valuenow={tenth(end)}
              aria-valuetext={preciseTime(end)}
              onpointerdown={(e) => (e.stopPropagation(), drag('end', e))}
              onkeydown={(e) => handleKey('end', e)}
            ></div>
          </div>
          {#each lanes as l (l.track)}
            <div
              class="lane"
              in:fade={{ duration: 200 }}
              title={app.t('trim.volumeHint')}
              role="slider"
              tabindex="0"
              aria-label={l.label}
              aria-valuemin={0}
              aria-valuemax={200}
              aria-valuenow={Math.round(l.gain * 100)}
              onpointerdown={(e) => laneDrag(l, e)}
              ondblclick={() => setGain(l, 1)}
              onkeydown={(e) => laneKey(l, e)}
              {@attach wheel(l)}
            >
              <Waveform peaks={peaks[l.track] ?? []} gain={l.gain} muted={l.muted} from={start / total} to={end / total} />
            </div>
          {/each}
          <div class="head" style:left="{(current / total) * 100}%"></div>
          </div>
        </div>
      {/if}
    </div>
  {/if}
</div>

{#if ghostSrc}
  <img class="fly-thumb" bind:this={ghostEl} src={ghostSrc} alt="" />
{/if}

<style>
  /* Not ".ghost": that's the ghost-button class used in the header. */
  .fly-thumb {
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
  /* Focused on open only to take focus into the dialog: no ring. */
  .viewer:focus-visible {
    outline: none;
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
  .send {
    min-width: 132px;
    height: 34px;
  }
  .send-in {
    display: inline-flex;
    align-items: center;
    gap: 8px;
  }
  .send.done {
    border-color: var(--accent);
    color: var(--accent);
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
    background: #0b0b0d;
  }
  /* Absolutely filled, so a 2560×1440 video can never size the layout. */
  .stage video,
  .stage img {
    position: absolute;
    inset: 0;
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
  .stage:hover .nav,
  .nav:focus-visible {
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
  /* Two stacked columns (names | strips) with matching row heights, so
     the playhead can run through the video strip and every lane. */
  .timeline {
    display: flex;
    gap: 10px;
    margin-top: 10px;
  }
  .heads {
    width: 150px;
    flex-shrink: 0;
  }
  .bodies {
    position: relative;
    flex: 1;
    min-width: 0;
  }
  .heads,
  .bodies {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .lane-head {
    height: 40px;
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    font-size: 12.5px;
    color: var(--text-2);
  }
  .video-head {
    height: 36px;
    padding-left: 6px;
    gap: 10px;
    color: var(--text-3);
  }
  .mute {
    flex-shrink: 0;
    width: 28px;
    height: 28px;
    display: grid;
    place-items: center;
    border-radius: var(--r-sm);
    border: 1px solid var(--line-2);
    background: var(--panel-2);
    color: var(--text-2);
    transition:
      color var(--dur-fast),
      background var(--dur-fast),
      border-color var(--dur-fast);
  }
  .mute:hover {
    color: var(--text);
    border-color: var(--text-3);
  }
  .mute.off {
    background: transparent;
    color: var(--text-3);
  }
  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .val {
    font-size: 11.5px;
    color: var(--text-3);
    padding-right: 2px;
  }
  .val.changed {
    color: var(--accent);
  }
  .lane {
    position: relative;
    height: 40px;
    border-radius: var(--r-sm);
    background: var(--bg);
    border: 1px solid var(--line-2);
    overflow: hidden;
    cursor: ns-resize;
    outline: none;
    touch-action: none;
  }
  .lane:focus-visible {
    border-color: var(--accent);
  }
  .track {
    position: relative;
    height: 36px;
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
    z-index: 3;
  }
  .handle {
    position: absolute;
    top: -1px;
    bottom: -1px;
    width: 10px;
    background: var(--accent);
    cursor: ew-resize;
  }
  .handle:focus-visible {
    outline-color: var(--text);
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
