<script lang="ts">
  import Icon from '$lib/components/Icon.svelte';
  import Switch from '$lib/components/Switch.svelte';
  import Keys from '$lib/components/Keys.svelte';
  import MediaCard from '$lib/components/MediaCard.svelte';
  import Viewer from '$lib/components/Viewer.svelte';
  import { api } from '$lib/api';
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
      <div class="txt">
        <h1>{on ? app.t('home.on') : app.t('home.off')}</h1>
        <p>{on ? (running ? readyLine(app.lang, s.replaySeconds) : app.t('home.starting')) : app.t('home.offHint')}</p>
      </div>
      {#if on}
        <Switch checked={on} label={app.t('set.replayEnabled')} onchange={(v) => api.setReplay(v)} />
      {:else}
        <button class="btn primary" onclick={() => api.setReplay(true)}>{app.t('home.turnOn')}</button>
      {/if}
    </div>

    {#if st?.lastError && on}
      <div class="err"><Icon name="alert" size={16} />{app.t('home.error')}: {st.lastError}</div>
    {:else if on && st && st.droppedFrames > 120}
      <div class="err warn"><Icon name="alert" size={16} />{app.t('home.dropped', { n: st.droppedFrames })}</div>
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
    padding: 22px 22px 20px;
    display: flex;
    flex-direction: column;
    gap: 22px;
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
    animation: pulse 2s ease-out infinite;
  }
  .dot.wait {
    background: var(--warn);
  }
  @keyframes pulse {
    0% {
      box-shadow: 0 0 0 0 color-mix(in srgb, var(--rec) 55%, transparent);
    }
    70%,
    100% {
      box-shadow: 0 0 0 9px transparent;
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
