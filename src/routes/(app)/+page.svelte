<script lang="ts">
  import IconCamera from '@tabler/icons-svelte-runes/icons/camera';
  import IconPlayerRecord from '@tabler/icons-svelte-runes/icons/player-record';
  import IconPlayerStop from '@tabler/icons-svelte-runes/icons/player-stop';
  import IconFolder from '@tabler/icons-svelte-runes/icons/folder';
  import IconBolt from '@tabler/icons-svelte-runes/icons/bolt';
  import IconArrowRight from '@tabler/icons-svelte-runes/icons/arrow-right';
  import IconAlertTriangle from '@tabler/icons-svelte-runes/icons/alert-triangle';
  import IconRocket from '@tabler/icons-svelte-runes/icons/rocket';
  import IconCpu from '@tabler/icons-svelte-runes/icons/cpu';
  import Ring from '$lib/components/Ring.svelte';
  import Switch from '$lib/components/Switch.svelte';
  import Keys from '$lib/components/Keys.svelte';
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
  let fill = $derived(on && st ? st.bufferSeconds / s.replaySeconds : 0);
  let recent = $derived(app.media.filter((m) => m.kind !== 'screenshot').slice(0, 8));
  let viewing = $state<MediaEntry | null>(null);
  let updating = $state(false);

  const encoderLabel = (e: string) =>
    e.includes('nvenc') ? 'NVENC' : e.includes('amf') ? 'AMD AMF' : e.includes('_mf') ? 'Media Foundation' : e.toUpperCase();

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
  <section class="hero card" class:off={!on}>
    <Ring value={fill} active={on && running} size={200} stroke={13}>
      {#if on && running}
        <div class="big">{duration(st?.bufferSeconds ?? 0)}</div>
        <div class="small">{app.t('home.of')} {duration(s.replaySeconds)}</div>
      {:else if on}
        <div class="small">{app.t('home.starting')}</div>
      {:else}
        <div class="big dim">OFF</div>
      {/if}
    </Ring>

    <div class="info">
      <div class="head">
        <div>
          <h1>{app.t('home.replay')}</h1>
          <p class="muted">{on ? app.t('home.replayOn') : app.t('home.replayOffHint')}</p>
        </div>
        <Switch big checked={on} label={app.t('home.replay')} onchange={(v) => api.setReplay(v)} />
      </div>

      {#if on && running && st}
        <div class="chips">
          <span class="chip"><IconCpu size={14} />{encoderLabel(st.encoder)} · {s.engine.codec.toUpperCase()}</span>
          <span class="chip">{st.width}×{st.height} · {Math.round(st.fps)} fps</span>
          <span class="chip">{app.t('home.memory')}: {bytes(st.bufferBytes, app.lang)}</span>
          {#if st.droppedFrames > 30}<span class="chip warn">{st.droppedFrames} {app.t('home.dropped')}</span>{/if}
        </div>
      {/if}
      {#if st?.lastError && on}
        <div class="err"><IconAlertTriangle size={16} />{app.t('home.error')}: {st.lastError}</div>
      {/if}

      <div class="actions">
        <button class="btn primary save" disabled={!on} onclick={() => api.saveClip()}>
          <IconBolt size={19} />
          {app.t('home.saveClip')}
          {#if s.hotkeys.saveClip}<span class="kb"><Keys accel={s.hotkeys.saveClip} /></span>{/if}
        </button>
        <button class="btn" onclick={() => api.screenshot()} title={s.hotkeys.screenshot}><IconCamera size={18} />{app.t('home.screenshot')}</button>
        <button class="btn" class:rec={st?.recording} onclick={() => api.toggleRecording()} title={s.hotkeys.toggleRecording}>
          {#if st?.recording}
            <IconPlayerStop size={18} />{app.t('home.stopRecord')} · {duration(st.recordingSeconds)}
          {:else}
            <IconPlayerRecord size={18} />{app.t('home.record')}
          {/if}
        </button>
        <button class="btn ghost icon" title={app.t('home.openFolder')} onclick={() => api.openMediaDir(false)}><IconFolder size={18} /></button>
      </div>
    </div>
  </section>

  {#if app.update}
    <div class="banner card">
      <IconRocket size={20} />
      <span>{app.t('home.update', { v: app.update.version })}</span>
      <span class="grow"></span>
      <button class="btn primary sm" disabled={updating} onclick={install}>{app.t('home.updateBtn')}</button>
    </div>
  {/if}
  {#if app.hotkeyErrors.length}
    <a class="banner card warnb" href="/settings#hotkeys">
      <IconAlertTriangle size={20} />
      <span>{app.t('home.hotkeyConflict')}</span>
    </a>
  {/if}

  <section class="recent">
    <div class="sec-head">
      <h2>{app.t('home.recent')}</h2>
      {#if app.media.length}
        <a class="btn ghost sm" href="/gallery">{app.t('home.showAll')}<IconArrowRight size={16} /></a>
      {/if}
    </div>
    {#if recent.length}
      <div class="grid">
        {#each recent as m (m.path)}
          <MediaCard entry={m} onopen={(e) => (viewing = e)} />
        {/each}
      </div>
    {:else if app.mediaLoaded}
      <div class="empty card">
        <div class="empty-icon"><IconBolt size={26} /></div>
        <div>
          <div class="empty-title">{app.t('home.empty')}</div>
          <div class="muted">{app.t('home.emptyHint', { key: s.hotkeys.saveClip.replaceAll('+', ' + '), min: Math.round(s.replaySeconds / 60) })}</div>
        </div>
      </div>
    {/if}
  </section>
</div>

{#if viewing}
  <Viewer entry={viewing} list={recent} onclose={() => (viewing = null)} onselect={(e) => (viewing = e)} />
{/if}

<style>
  .page {
    max-width: 1180px;
    margin: 0 auto;
    display: flex;
    flex-direction: column;
    gap: 22px;
  }
  .hero {
    display: flex;
    align-items: center;
    gap: 40px;
    padding: 30px 34px;
    position: relative;
    overflow: hidden;
  }
  .hero::after {
    content: '';
    position: absolute;
    inset: 0;
    border-radius: inherit;
    padding: 1px;
    background: linear-gradient(135deg, color-mix(in srgb, var(--accent-a) 45%, transparent), transparent 40%, transparent 60%, color-mix(in srgb, var(--accent-b) 40%, transparent));
    -webkit-mask:
      linear-gradient(#000 0 0) content-box,
      linear-gradient(#000 0 0);
    -webkit-mask-composite: xor;
    mask-composite: exclude;
    pointer-events: none;
  }
  .hero.off::after {
    opacity: 0.3;
  }
  .big {
    font-family: var(--font-display);
    font-size: 40px;
    font-weight: 600;
    letter-spacing: -0.02em;
    line-height: 1.1;
  }
  .big.dim {
    color: var(--text-3);
    font-size: 30px;
  }
  .small {
    color: var(--text-2);
    font-size: 13px;
    font-weight: 600;
    margin-top: 2px;
  }
  .info {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 20px;
  }
  h1 {
    font-size: 26px;
  }
  .head p {
    margin: 6px 0 0;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }
  .chip.warn {
    color: var(--warn);
  }
  .err {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--danger);
    font-size: 13px;
    font-weight: 600;
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
    margin-top: 4px;
  }
  .save {
    height: 46px;
    padding: 0 20px;
    font-size: 15px;
  }
  .save:disabled {
    opacity: 0.45;
    cursor: default;
    box-shadow: none;
  }
  .save .kb :global(kbd) {
    background: rgba(27, 16, 48, 0.18);
    border-color: rgba(27, 16, 48, 0.25);
    color: var(--accent-ink);
  }
  .save .kb :global(.plus) {
    color: var(--accent-ink);
  }
  .actions .btn:not(.save) {
    height: 46px;
  }
  .btn.rec {
    color: #ff6b8b;
    border-color: rgba(255, 107, 139, 0.4);
  }
  .banner {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 12px 16px;
    font-weight: 600;
    color: inherit;
    text-decoration: none;
  }
  .warnb {
    color: var(--warn);
  }
  .grow {
    flex: 1;
  }
  .sec-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 14px;
  }
  h2 {
    font-size: 17px;
  }
  .sec-head a {
    text-decoration: none;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(230px, 1fr));
    gap: 14px;
  }
  .empty {
    display: flex;
    align-items: center;
    gap: 18px;
    padding: 24px;
  }
  .empty-icon {
    width: 54px;
    height: 54px;
    border-radius: 16px;
    display: grid;
    place-items: center;
    background: var(--accent-soft);
    color: var(--accent-a);
  }
  .empty-title {
    font-weight: 700;
    font-size: 15px;
  }
</style>
