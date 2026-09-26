<script lang="ts">
  // In-game menu (Alt+X): a transparent window over the game. Left column:
  // save / quick actions / recent replays (or quick settings) / hardware load.
  // The backend shows the window and sends menu://open; closing plays the
  // animation first, then asks the backend to hide it and refocus the game.
  import { onMount } from 'svelte';
  import { listen } from '@tauri-apps/api/event';
  import Icon, { type IconName } from '$lib/components/Icon.svelte';
  import Logo from '$lib/components/Logo.svelte';
  import Keys from '$lib/components/Keys.svelte';
  import Slider from '$lib/components/Slider.svelte';
  import Segmented from '$lib/components/Segmented.svelte';
  import Switch from '$lib/components/Switch.svelte';
  import HotkeyInput from '$lib/components/HotkeyInput.svelte';
  import { api, fileUrl } from '$lib/api';
  import { app } from '$lib/app.svelte';
  import { ago, bytes, duration } from '$lib/format';
  import type { Hotkeys, MediaEntry, SystemStats } from '$lib/types';
  import type { TKey } from '$lib/i18n';

  let shown = $state(false);
  let view = $state<'clips' | 'settings'>('clips');
  let stats = $state<SystemStats | null>(null);
  let thumbs = $state<Record<string, string>>({});
  let copied = $state<string | null>(null);
  let savedFlash = $state(false);
  let shotFlash = $state(false);
  let statsTimer: ReturnType<typeof setInterval> | undefined;

  let s = $derived(app.settings);
  let st = $derived(app.status);
  let recent = $derived(app.media.filter((m) => m.kind !== 'screenshot').slice(0, 12));

  async function pollStats() {
    stats = await api.systemStats().catch(() => stats);
  }

  function open() {
    view = 'clips';
    shown = true;
    app.refreshMedia();
    pollStats();
    clearInterval(statsTimer);
    statsTimer = setInterval(pollStats, 1000);
  }

  let closing = false;
  async function close() {
    if (closing || !shown) return;
    closing = true;
    shown = false;
    clearInterval(statsTimer);
    await new Promise((r) => setTimeout(r, 220));
    await api.menuClose().catch(() => {});
    closing = false;
  }

  onMount(() => {
    const offs = [listen('menu://open', open), listen('menu://close', close)];
    (async () => {
      if (import.meta.env.DEV && !('__TAURI_INTERNALS__' in window)) {
        (await import('$lib/mock')).installMock();
        // Browser preview: something game-like behind the menu.
        document.body.style.background = 'url(/dev-clip-thumb.jpg) center / cover';
      }
      await app.init();
      // First open: the window was created for it, so the event came too early.
      open();
    })();
    return () => {
      clearInterval(statsTimer);
      offs.forEach((p) => p.then((off) => off()));
    };
  });

  // Newly listed clips get their thumbnails.
  $effect(() => {
    for (const m of recent) {
      if (!(m.path in thumbs)) {
        thumbs[m.path] = '';
        api.thumbnail(m.path).then((t) => (thumbs[m.path] = fileUrl(t))).catch(() => {});
      }
    }
  });

  // "Saved" on the big button when a clip lands.
  let lastSaved = 0;
  $effect(() => {
    const at = app.clipSavedAt;
    if (at && at !== lastSaved && shown) {
      lastSaved = at;
      savedFlash = true;
      setTimeout(() => (savedFlash = false), 1500);
    }
  });

  async function screenshot() {
    // The menu is excluded from capture, so it never shows up in the shot.
    await api.screenshot();
    shotFlash = true;
    setTimeout(() => (shotFlash = false), 1200);
  }

  async function send(m: MediaEntry) {
    try {
      await api.copyMedia([m.path]);
      copied = m.path;
      setTimeout(() => copied === m.path && (copied = null), 1800);
    } catch (e) {
      app.notify(String(e), 'error');
    }
  }

  const minutes = (sec: number) => `${Math.floor(sec / 60)}:${String(sec % 60).padStart(2, '0')}`;
  const hkRows: { key: keyof Hotkeys; label: TKey }[] = [
    { key: 'saveClip', label: 'hk.saveClip' },
    { key: 'saveShort', label: 'hk.saveShort' },
    { key: 'screenshot', label: 'hk.screenshot' },
    { key: 'toggleRecording', label: 'hk.toggleRecording' },
    { key: 'toggleReplay', label: 'hk.toggleReplay' },
    { key: 'toggleMenu', label: 'hk.toggleMenu' },
  ];

  type Action = { icon: IconName; label: string; on?: boolean; alert?: boolean; run: () => void };
  let actions = $derived<Action[]>(
    s && st
      ? [
          { icon: shotFlash ? 'check' : 'shot', label: app.t('home.screenshot'), run: screenshot },
          { icon: st.recording ? 'stop' : 'record', label: app.t(st.recording ? 'home.stopRecord' : 'home.record'), on: st.recording, run: () => api.toggleRecording() },
          {
            icon: s.engine.micMuted ? 'micOff' : 'mic',
            label: app.t('menu.mic'),
            alert: s.engine.micMuted || !s.engine.mic,
            run: () => app.change((x) => (x.engine.micMuted = !x.engine.micMuted), 0),
          },
          { icon: 'replay', label: app.t('menu.replay'), on: s.replayEnabled, alert: !s.replayEnabled, run: () => api.setReplay(!s.replayEnabled) },
          { icon: 'sliders', label: app.t('nav.settings'), on: view === 'settings', run: () => (view = view === 'settings' ? 'clips' : 'settings') },
        ]
      : [],
  );
</script>

<svelte:window onkeydown={(e) => e.key === 'Escape' && close()} />

<div class="menu" class:shown>
  <div class="dim" onclick={close} role="presentation"></div>
  {#if s && st}
    <aside class="panel">
      <header>
        <Logo size={30} />
        <span class="name">GeniusClip</span>
        <span class="status" class:off={!s.replayEnabled}><i></i>{app.t(s.replayEnabled ? 'menu.replayOn' : 'menu.replayOff')}</span>
      </header>

      <button class="save on-accent" class:saved={savedFlash} disabled={!s.replayEnabled} onclick={() => api.saveClip()}>
        <Icon name={savedFlash ? 'check' : 'clapper'} size={19} stroke={1.9} />
        <span>{app.t(savedFlash ? 'home.saved' : 'home.saveClip')}</span>
        {#if s.hotkeys.saveClip}<Keys variant="chip" accel={s.hotkeys.saveClip} />{/if}
      </button>

      <div class="actions">
        {#each actions as a (a.label)}
          <button class="act" class:on={a.on} class:alert={a.alert} onclick={a.run} title={a.label}>
            <Icon name={a.icon} size={20} />
            <span>{a.label}</span>
          </button>
        {/each}
      </div>

      {#if view === 'clips'}
        <section class="list">
          <h4>{app.t('menu.recent')}</h4>
          {#if recent.length === 0}
            <p class="empty">{app.t('menu.empty')}</p>
          {/if}
          {#each recent as m (m.path)}
            <div class="clip" class:fresh={app.fresh[m.path]}>
              <button class="thumb" onclick={() => api.openInApp('gallery', m.path)} title={app.t('menu.open')}>
                {#if thumbs[m.path]}<img src={thumbs[m.path]} alt="" />{/if}
                <span class="dur mono">{duration(m.duration)}</span>
              </button>
              <div class="meta">
                <b>{m.game || m.name}</b>
                <span>{ago(m.modified, app.lang)} · {bytes(m.size, app.lang)}</span>
              </div>
              <div class="tools">
                <button title={app.t('gallery.send')} onclick={() => send(m)}><Icon name={copied === m.path ? 'check' : 'send'} size={16} /></button>
                <button title={app.t('menu.open')} onclick={() => api.openInApp('gallery', m.path)}><Icon name="open" size={16} /></button>
                <button title={app.t('gallery.reveal')} onclick={() => api.revealPath(m.path)}><Icon name="folder" size={16} /></button>
              </div>
            </div>
          {/each}
        </section>
      {:else}
        <section class="list quick">
          <h4>
            <button class="back" onclick={() => (view = 'clips')} title={app.t('menu.back')}><Icon name="left" size={16} /></button>
            {app.t('nav.settings')}
          </h4>
          <div class="q">
            <span>{app.t('set.length')}</span>
            <Slider
              value={s.replaySeconds}
              min={60}
              max={s.engine.diskBuffer ? 3600 : 1200}
              step={30}
              format={minutes}
              width="100%"
              onchange={(v) => app.change((x) => (x.replaySeconds = v), 600)}
            />
          </div>
          <div class="q">
            <span>{app.t('set.quality')}</span>
            <Segmented
              value={s.engine.quality}
              onchange={(v) => app.change((x) => (x.engine.quality = v))}
              options={(['low', 'medium', 'high', 'ultra'] as const).map((q) => ({ value: q, label: app.t(`set.q.${q}`) }))}
            />
          </div>
          <div class="q">
            <span>{app.t('menu.gameVolume')}</span>
            <Slider value={Math.round(s.engine.systemVolume * 100)} min={0} max={200} step={5} format={(v) => `${v}%`} width="100%" onchange={(v) => app.change((x) => (x.engine.systemVolume = v / 100), 300)} />
          </div>
          <div class="q">
            <span>{app.t('menu.micVolume')}</span>
            <Slider value={Math.round(s.engine.micVolume * 100)} min={0} max={300} step={5} format={(v) => `${v}%`} width="100%" onchange={(v) => app.change((x) => (x.engine.micVolume = v / 100), 300)} />
          </div>
          <div class="q row">
            <span>{app.t('set.noise')}</span>
            <Switch checked={s.engine.noiseSuppression} onchange={(v) => app.change((x) => (x.engine.noiseSuppression = v), 0)} />
          </div>
          <div class="q row">
            <span>{app.t('set.overlayEnabled')}</span>
            <Switch checked={s.overlay.enabled} onchange={(v) => app.change((x) => (x.overlay.enabled = v), 0)} />
          </div>
          <h5>{app.t('set.hotkeys')}</h5>
          {#each hkRows as r (r.key)}
            <div class="q row hk">
              <span>{app.t(r.label)}</span>
              <HotkeyInput value={s.hotkeys[r.key]} conflict={app.hotkeyErrors.includes(r.key)} onchange={(v) => app.change((x) => (x.hotkeys[r.key] = v), 0)} />
            </div>
          {/each}
          <button class="all" onclick={() => api.openInApp('settings')}>{app.t('menu.allSettings')}<Icon name="right" size={15} /></button>
        </section>
      {/if}

      <footer>
        <div class="stats mono">
          {#if stats}
            {#if stats.gpu !== null}<span><b>GPU</b> {Math.round(stats.gpu)}%{#if stats.gpuTemp !== null} · {stats.gpuTemp}°{/if}</span>{/if}
            <span><b>CPU</b> {Math.round(stats.cpu)}%</span>
            <span><b>{app.t('home.ram')}</b> {stats.ramUsedGb.toFixed(1)}/{Math.round(stats.ramTotalGb)}</span>
          {/if}
        </div>
      </footer>
    </aside>
    <span class="hint">{app.t('menu.close')}</span>
  {/if}
</div>

<style>
  .menu {
    position: fixed;
    inset: 0;
    overflow: hidden;
  }
  .dim {
    position: absolute;
    inset: 0;
    background: rgba(6, 6, 8, 0.5);
    opacity: 0;
    transition: opacity 0.22s var(--ease);
  }
  .shown .dim {
    opacity: 1;
  }
  .panel {
    position: absolute;
    top: 0;
    bottom: 0;
    left: 0;
    width: 360px;
    display: flex;
    flex-direction: column;
    background: rgba(18, 18, 21, 0.96);
    border-right: 1px solid var(--line-2);
    box-shadow: 24px 0 80px -20px rgba(0, 0, 0, 0.7);
    transform: translateX(-40px);
    opacity: 0;
    transition:
      transform 0.28s var(--ease),
      opacity 0.2s var(--ease);
  }
  .panel::before {
    /* Accent glow in the corner, like the home hero. */
    content: '';
    position: absolute;
    inset: 0 0 auto;
    height: 220px;
    background: radial-gradient(260px 160px at 0 0, color-mix(in srgb, var(--accent) 22%, transparent), transparent 70%);
    pointer-events: none;
  }
  .shown .panel {
    transform: none;
    opacity: 1;
  }
  header {
    position: relative;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 20px 20px 14px;
  }
  .name {
    font-weight: 650;
    font-size: 16px;
  }
  .status {
    margin-left: auto;
    display: flex;
    align-items: center;
    gap: 7px;
    font-size: 12.5px;
    color: var(--text-2);
  }
  .status i {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--rec);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--rec) 25%, transparent);
  }
  .status.off i {
    background: var(--text-3);
    box-shadow: none;
  }
  .save {
    position: relative;
    margin: 4px 20px 0;
    height: 52px;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 10px;
    border: 0;
    border-radius: 14px;
    background: var(--accent-grad);
    color: var(--accent-ink);
    font-size: 15.5px;
    font-weight: 650;
    cursor: pointer;
    transition:
      filter 0.15s,
      transform 0.15s var(--ease);
  }
  .save:hover {
    filter: brightness(1.07);
  }
  .save:active {
    transform: scale(0.985);
  }
  .save:disabled {
    filter: grayscale(0.8) brightness(0.6);
    cursor: default;
  }
  .actions {
    position: relative;
    display: grid;
    grid-template-columns: repeat(5, 1fr);
    gap: 4px;
    padding: 12px 14px 8px;
  }
  .act {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    padding: 10px 2px 8px;
    border: 1px solid transparent;
    border-radius: 12px;
    background: none;
    color: var(--text-2);
    font-size: 11.5px;
    cursor: pointer;
    transition:
      background 0.15s,
      color 0.15s;
  }
  .act span {
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .act:hover {
    background: var(--hover);
    color: var(--text);
  }
  .act.on {
    color: var(--accent);
    background: var(--accent-soft);
  }
  .act.alert {
    color: var(--danger);
  }
  .list {
    position: relative;
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 6px 12px 12px;
    border-top: 1px solid var(--line);
  }
  h4 {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 10px 8px 8px;
    font-size: 12px;
    font-weight: 600;
    color: var(--text-3);
    text-transform: none;
  }
  h5 {
    margin: 18px 8px 4px;
    font-size: 12px;
    font-weight: 600;
    color: var(--text-3);
  }
  .empty {
    margin: 24px 8px;
    color: var(--text-3);
    font-size: 13px;
    text-align: center;
  }
  .clip {
    position: relative;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 7px 8px;
    border-radius: 12px;
    transition: background 0.15s;
  }
  .clip:hover {
    background: var(--hover);
  }
  .clip.fresh {
    background: var(--accent-soft);
  }
  .thumb {
    position: relative;
    flex: none;
    width: 112px;
    height: 63px;
    padding: 0;
    border: 0;
    border-radius: 8px;
    overflow: hidden;
    background: var(--panel-2);
    cursor: pointer;
  }
  .thumb img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .dur {
    position: absolute;
    right: 5px;
    bottom: 4px;
    padding: 0 5px;
    border-radius: 4px;
    background: rgba(0, 0, 0, 0.65);
    color: #fff;
    font-size: 10.5px;
  }
  .meta {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .meta b {
    font-size: 13.5px;
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .meta span {
    font-size: 12px;
    color: var(--text-3);
  }
  .tools {
    position: absolute;
    right: 8px;
    top: 50%;
    display: flex;
    gap: 2px;
    padding: 3px;
    border-radius: 10px;
    background: rgba(24, 24, 28, 0.95);
    border: 1px solid var(--line-2);
    transform: translateY(-50%);
    opacity: 0;
    transition: opacity 0.15s;
  }
  .clip:hover .tools {
    opacity: 1;
  }
  .tools button {
    width: 30px;
    height: 30px;
    display: grid;
    place-items: center;
    border: 0;
    border-radius: 8px;
    background: none;
    color: var(--text-2);
    cursor: pointer;
  }
  .tools button:hover {
    background: var(--hover);
    color: var(--text);
  }
  .back {
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    margin-left: -6px;
    border: 0;
    border-radius: 7px;
    background: none;
    color: var(--text-2);
    cursor: pointer;
  }
  .back:hover {
    background: var(--hover);
    color: var(--text);
  }
  .q {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px 8px;
    font-size: 13px;
    color: var(--text-2);
  }
  .q.row {
    flex-direction: row;
    align-items: center;
    justify-content: space-between;
  }
  .q.hk :global(.field) {
    min-width: 150px;
  }
  .all {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 14px 8px 4px;
    padding: 0;
    border: 0;
    background: none;
    color: var(--accent);
    font-size: 13.5px;
    font-weight: 550;
    cursor: pointer;
  }
  footer {
    position: relative;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 12px 20px 16px;
    border-top: 1px solid var(--line);
    font-size: 11.5px;
    color: var(--text-3);
  }
  .stats {
    display: flex;
    gap: 16px;
    white-space: nowrap;
    font-size: 11.5px;
    color: var(--text-2);
  }
  .stats b {
    font-weight: 600;
    color: var(--text-3);
  }
  /* Floating over the game, top right. */
  .hint {
    position: absolute;
    top: 20px;
    right: 24px;
    padding: 7px 12px;
    border-radius: 10px;
    background: rgba(18, 18, 21, 0.85);
    border: 1px solid var(--line-2);
    font-size: 12.5px;
    color: var(--text-2);
    opacity: 0;
    transition: opacity 0.25s var(--ease);
  }
  .shown .hint {
    opacity: 1;
  }
</style>
