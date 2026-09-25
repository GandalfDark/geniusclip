<script lang="ts">
  import Icon from '$lib/components/Icon.svelte';
  import Switch from '$lib/components/Switch.svelte';
  import Keys from '$lib/components/Keys.svelte';
  import Timeline from '$lib/components/Timeline.svelte';
  import MediaCard from '$lib/components/MediaCard.svelte';
  import Viewer from '$lib/components/Viewer.svelte';
  import { api } from '$lib/api';
  import { app } from '$lib/app.svelte';
  import { bytes, duration } from '$lib/format';
  import type { MediaEntry } from '$lib/types';

  let s = $derived(app.settings!);
  let st = $derived(app.status);
  let on = $derived(s.replayEnabled);
  let running = $derived(!!st?.running);
  let buffered = $derived(on && st ? Math.min(st.bufferSeconds, s.replaySeconds) : 0);
  let recent = $derived(app.media.filter((m) => m.kind !== 'screenshot').slice(0, 8));
  let viewing = $state<MediaEntry | null>(null);
  let updating = $state(false);

  const encoderLabel = (e: string) => (e.includes('nvenc') ? 'NVENC' : e.includes('amf') ? 'AMF' : e.includes('_mf') ? 'Media Foundation' : e.toUpperCase());
  const codecLabel = (c: string) => ({ h264: 'H.264', hevc: 'HEVC', av1: 'AV1' })[c] ?? c;

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
    <div class="status">
      {#if on && running && st}
        <span class="dot"></span>
        <span class="mono">
          {app.t('home.rec')} · {st.width}×{st.height} · {Math.round(st.fps)} {app.t('home.fps')} · {encoderLabel(st.encoder)} {codecLabel(s.engine.codec)} · {bytes(st.bufferBytes, app.lang)} {app.t('home.ram')}
        </span>
        {#if st.droppedFrames > 30}<span class="warn mono">· {app.t('home.dropped', { n: st.droppedFrames })}</span>{/if}
      {:else if on}
        <span class="mono muted">{app.t('home.starting')}</span>
      {:else}
        <span class="mono muted">{app.t('home.off')}</span>
      {/if}
      <span class="grow"></span>
      <Switch checked={on} label={app.t('set.replayEnabled')} onchange={(v) => api.setReplay(v)} />
    </div>

    {#if on}
      <div class="clock mono">
        <span class="now">{duration(buffered)}</span><span class="of">/ {duration(s.replaySeconds)}</span>
      </div>
      <Timeline seconds={buffered} total={s.replaySeconds} live={running} />
    {:else}
      <p class="off-hint">{app.t('home.offHint')}</p>
    {/if}

    {#if st?.lastError && on}
      <div class="err"><Icon name="alert" size={16} />{app.t('home.error')}: {st.lastError}</div>
    {/if}

    <div class="actions">
      <button class="btn primary big" disabled={!on} onclick={() => api.saveClip()}>
        <Icon name="save" size={17} stroke={2} />{app.t('home.saveClip')}{#if s.hotkeys.saveClip}<Keys plain accel={s.hotkeys.saveClip} />{/if}
      </button>
      <button class="btn big" onclick={() => api.screenshot()}>
        <Icon name="shot" size={17} />{app.t('home.screenshot')}{#if s.hotkeys.screenshot}<Keys plain accel={s.hotkeys.screenshot} />{/if}
      </button>
      <button class="btn big" class:recording={st?.recording} onclick={() => api.toggleRecording()}>
        {#if st?.recording}
          <Icon name="stop" size={17} />{app.t('home.stopRecord')}<span class="mono rec-time">{duration(st.recordingSeconds)}</span>
        {:else}
          <Icon name="record" size={17} />{app.t('home.record')}{#if s.hotkeys.toggleRecording}<Keys plain accel={s.hotkeys.toggleRecording} />{/if}
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
        {#each recent as m (m.path)}
          <MediaCard entry={m} onopen={(e) => (viewing = e)} />
        {/each}
      </div>
    {:else if app.mediaLoaded}
      <p class="muted empty">{app.t('home.empty', { key: s.hotkeys.saveClip.replaceAll('+', ' + '), min: Math.round(s.replaySeconds / 60) })}</p>
    {/if}
  </section>
</div>

{#if viewing}
  <Viewer entry={viewing} list={recent} onclose={() => (viewing = null)} onselect={(e) => (viewing = e)} />
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
    padding: 18px 20px 20px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .status {
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: 12px;
    color: var(--text-2);
    min-height: 20px;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--rec);
    flex-shrink: 0;
    animation: blink 1.6s steps(1) infinite;
  }
  @keyframes blink {
    50% {
      opacity: 0.35;
    }
  }
  .warn {
    color: var(--warn);
  }
  .grow {
    flex: 1;
  }
  .clock {
    display: flex;
    align-items: baseline;
    gap: 10px;
    line-height: 1;
    margin-top: 4px;
  }
  .now {
    font-size: 44px;
    font-weight: 700;
    letter-spacing: -1.5px;
  }
  .of {
    font-size: 16px;
    color: var(--text-3);
  }
  .off-hint {
    margin: 6px 0 2px;
    color: var(--text-2);
  }
  .err {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--danger);
    font-size: 12.5px;
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 4px;
  }
  .big {
    height: 38px;
    padding: 0 14px;
    font-size: 13.5px;
  }
  .big.icon {
    width: 38px;
    padding: 0;
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
