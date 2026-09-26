<script lang="ts">
  import Icon from '$lib/components/Icon.svelte';
  import Switch from '$lib/components/Switch.svelte';
  import Keys from '$lib/components/Keys.svelte';
  import MediaCard from '$lib/components/MediaCard.svelte';
  import Viewer from '$lib/components/Viewer.svelte';
  import { flip } from 'svelte/animate';
  import { api } from '$lib/api';
  import { enter } from '$lib/enter';
  import { flipParams, leave, rise } from '$lib/motion';
  import type { Origin } from '$lib/components/Viewer.svelte';
  import { app } from '$lib/app.svelte';
  import { duration } from '$lib/format';
  import { readyLine } from '$lib/i18n';
  import type { MediaEntry } from '$lib/types';

  let s = $derived(app.settings!);
  let st = $derived(app.status);
  let on = $derived(s.replayEnabled);
  let running = $derived(!!st?.running);
  let recent = $derived(app.media.filter((m) => m.kind !== 'screenshot').slice(0, 8));
  let viewing = $state<MediaEntry | null>(null);
  let origin = $state<Origin | null>(null);
  let justSaved = $state(false);

  // The main button confirms each saved clip for a moment (hotkey saves too).
  $effect(() => {
    if (!app.clipSavedAt) return;
    justSaved = true;
    const t = setTimeout(() => (justSaved = false), 1500);
    return () => clearTimeout(t);
  });

  function open(e: MediaEntry, rect: DOMRect, src: string | null) {
    origin = { rect, src };
    viewing = e;
  }
  let updating = $state(false);


  async function install() {
    updating = true;
    try {
      await api.installUpdate();
    } catch (e) {
      app.notify(String(e), 'error');
      updating = false;
    }
  }
</script>

<div class="page">
  <section class="deck panel" class:off={!on}>
    <div class="head">
      <span class="dot" class:live={on && running} class:wait={on && !running}></span>
      <!-- Animate only the on/off change; capture start-up is shown by the dot. -->
      {#key on}
        <div class="txt" in:rise={{ y: 6 }}>
          <h1>{on ? app.t('home.on') : app.t('home.off')}</h1>
          <p>{on ? readyLine(app.lang, s.replaySeconds) : app.t('home.offHint')}</p>
        </div>
      {/key}
      {#if on}
        <Switch checked={on} label={app.t('set.replayEnabled')} onchange={(v) => api.setReplay(v)} />
      {:else}
        <button class="btn primary" onclick={() => api.setReplay(true)}>{app.t('home.turnOn')}</button>
      {/if}
    </div>

    {#if st?.lastError && on}
      <div class="err"><Icon name="alert" size={16} />{app.t('home.error')}: {st.lastError}</div>
    {:else if on && st && st.droppedRecent > 30}
      <!-- Only while frames are being lost right now (last 10 s), not a lifetime total. -->
      <div class="err warn"><Icon name="alert" size={16} />{app.t('home.dropped', { n: st.droppedRecent })}</div>
    {/if}

    <div class="actions">
      <button class="btn primary big save" class:saved={justSaved} disabled={!on} onclick={() => api.saveClip()}>
        {#key justSaved}
          <span class="save-in" in:rise={{ y: 6, duration: 260 }}>
            {#if justSaved}
              <Icon name="check" size={17} stroke={2.2} />{app.t('home.saved')}
            {:else}
              <Icon name="clapper" size={17} stroke={1.9} />{app.t('home.saveClip')}{#if s.hotkeys.saveClip}<Keys variant="chip" accel={s.hotkeys.saveClip} />{/if}
            {/if}
          </span>
        {/key}
      </button>
      <button class="btn big" onclick={() => api.screenshot()}>
        <Icon name="shot" size={17} />{app.t('home.screenshot')}{#if s.hotkeys.screenshot}<Keys variant="chip" accel={s.hotkeys.screenshot} />{/if}
      </button>
      <button class="btn big" class:recording={st?.recording} onclick={() => api.toggleRecording()}>
        {#if st?.recording}
          <Icon name="stop" size={17} />{app.t('home.stopRecord')}<span class="mono rec-time">{duration(st.recordingSeconds)}</span>
        {:else}
          <Icon name="record" size={17} />{app.t('home.record')}{#if s.hotkeys.toggleRecording}<Keys variant="chip" accel={s.hotkeys.toggleRecording} />{/if}
        {/if}
      </button>
      <span class="grow"></span>
      <button class="btn ghost big icon" title={app.t('home.openFolder')} onclick={() => api.openMediaDir(false)}><Icon name="folder" size={18} /></button>
    </div>
  </section>

  {#if app.update}
    <div class="note panel">
      <Icon name="update" size={18} />
      <span>{app.t('home.update', { v: app.update.version })}</span>
      <span class="grow"></span>
      <button class="btn primary sm" disabled={updating} onclick={install}>{app.t('home.updateBtn')}</button>
    </div>
  {/if}
  {#if app.hotkeyErrors.length}
    <a class="note panel warnnote" href="/settings#hotkeys">
      <Icon name="alert" size={18} /><span>{app.t('home.hotkeyConflict')}</span>
    </a>
  {/if}

  <section>
    <div class="sec-head">
      <h2>{app.t('home.recent')}</h2>
      {#if app.media.length}
        <a class="link" href="/gallery">{app.t('home.gallery')}<Icon name="right" size={15} stroke={2} /></a>
      {/if}
    </div>
    {#if recent.length}
      <div class="grid">
        {#each recent as m, i (m.path)}
          <div class="cell" animate:flip={flipParams()} in:enter|global={{ i, fresh: !!app.fresh[m.path] }} out:leave>
            <MediaCard entry={m} onopen={open} />
          </div>
        {/each}
      </div>
    {:else if app.mediaLoaded}
      <p class="muted empty">{app.t('home.empty', { key: s.hotkeys.saveClip.replaceAll('+', ' + '), min: Math.round(s.replaySeconds / 60) })}</p>
    {/if}
  </section>
</div>

{#if viewing}
  <Viewer entry={viewing} list={recent} {origin} onclose={() => (viewing = null)} onselect={(e) => (viewing = e)} />
{/if}

<style>
  .page {
    max-width: 1120px;
    margin: 0 auto;
    display: flex;
    flex-direction: column;
    gap: 24px;
    padding-top: 4px;
  }
  .deck {
    position: relative;
    overflow: hidden;
    padding: 24px 24px 22px;
    display: flex;
    flex-direction: column;
    gap: 24px;
    border-radius: 14px;
  }
  /* Soft accent glow in the corner; fades out when replay is off. */
  .deck::before {
    content: '';
    position: absolute;
    left: -90px;
    top: -140px;
    width: 420px;
    height: 300px;
    border-radius: 50%;
    background: radial-gradient(circle, color-mix(in srgb, var(--accent) 24%, transparent), transparent 66%);
    pointer-events: none;
    transition: opacity 0.4s;
  }
  .deck.off::before {
    opacity: 0.25;
  }
  .head,
  .actions,
  .err {
    position: relative;
  }
  .head {
    display: flex;
    align-items: flex-start;
    gap: 16px;
  }
  .dot {
    width: 10px;
    height: 10px;
    margin-top: 11px;
    border-radius: 50%;
    background: var(--text-3);
    flex-shrink: 0;
  }
  .dot.live {
    background: var(--rec);
    box-shadow: 0 0 0 5px color-mix(in srgb, var(--rec) 16%, transparent);
    animation: pulse 2.2s ease-out infinite;
  }
  .dot.wait {
    background: var(--warn);
  }
  .dot {
    transition: background var(--dur) var(--ease);
  }
  @keyframes pulse {
    0% {
      box-shadow: 0 0 0 0 color-mix(in srgb, var(--rec) 50%, transparent);
    }
    70%,
    100% {
      box-shadow: 0 0 0 10px transparent;
    }
  }
  .txt {
    flex: 1;
    min-width: 0;
  }
  h1 {
    font-size: 26px;
    font-weight: 600;
    letter-spacing: -0.4px;
    line-height: 1.25;
  }
  .off h1 {
    color: var(--text-2);
  }
  .txt p {
    margin: 4px 0 0;
    font-size: 14px;
    color: var(--text-2);
  }
  .grow {
    flex: 1;
  }
  .err {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: -8px;
    color: var(--danger);
    font-size: 12.5px;
  }
  .err.warn {
    color: var(--warn);
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .big {
    height: 42px;
    padding: 0 15px;
    border-radius: 10px;
    font-size: 13.5px;
    font-weight: 600;
    gap: 9px;
  }
  .big:not(.primary):not(.ghost) {
    background: var(--hover);
  }
  .big:not(.primary):not(.ghost):hover {
    background: #2a2a31;
  }
  .big.icon {
    width: 42px;
    padding: 0;
  }
  .save-in {
    display: inline-flex;
    align-items: center;
    gap: 9px;
  }
  .save.saved {
    box-shadow: 0 0 0 4px color-mix(in srgb, var(--accent) 25%, transparent);
  }
  .cell {
    min-width: 0;
  }
  .recording {
    border-color: color-mix(in srgb, var(--rec) 60%, transparent);
  }
  .rec-time {
    font-size: 12px;
    color: var(--rec);
  }
  .note {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 14px;
    font-size: 13px;
    color: inherit;
    text-decoration: none;
  }
  .warnnote {
    color: var(--warn);
  }
  .sec-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 12px;
  }
  h2 {
    font-size: 14px;
    font-weight: 600;
  }
  .link {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    font-size: 12.5px;
    color: var(--text-2);
    text-decoration: none;
  }
  .link:hover {
    color: var(--text);
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(210px, 1fr));
    gap: 18px 14px;
  }
  .empty {
    margin: 0;
    font-size: 13px;
  }
</style>
