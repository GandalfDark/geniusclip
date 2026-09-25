<script lang="ts">
  import { open as openDialog } from '@tauri-apps/plugin-dialog';
  import IconVideo from '@tabler/icons-svelte-runes/icons/video';
  import IconVolume from '@tabler/icons-svelte-runes/icons/volume';
  import IconKeyboard from '@tabler/icons-svelte-runes/icons/keyboard';
  import IconFolder from '@tabler/icons-svelte-runes/icons/folder';
  import IconBell from '@tabler/icons-svelte-runes/icons/bell';
  import IconPalette from '@tabler/icons-svelte-runes/icons/palette';
  import IconPower from '@tabler/icons-svelte-runes/icons/power';
  import IconCheck from '@tabler/icons-svelte-runes/icons/check';
  import IconRefresh from '@tabler/icons-svelte-runes/icons/refresh';
  import Row from '$lib/components/Row.svelte';
  import Switch from '$lib/components/Switch.svelte';
  import Segmented from '$lib/components/Segmented.svelte';
  import Select from '$lib/components/Select.svelte';
  import Slider from '$lib/components/Slider.svelte';
  import HotkeyInput from '$lib/components/HotkeyInput.svelte';
  import Logo from '$lib/components/Logo.svelte';
  import { api } from '$lib/api';
  import { app } from '$lib/app.svelte';
  import { ACCENTS } from '$lib/accents';
  import type { Estimate, Hotkeys } from '$lib/types';
  import { invoke } from '@tauri-apps/api/core';

  let s = $derived(app.settings!);
  let snap = $derived(app.snapshot!);
  let estimate = $state<Estimate | null>(null);
  let checking = $state(false);
  let active = $state('capture');
  let showSaved = $state(false);

  const sections = [
    { id: 'capture', icon: IconVideo, key: 'set.capture' as const },
    { id: 'audio', icon: IconVolume, key: 'set.audio' as const },
    { id: 'hotkeys', icon: IconKeyboard, key: 'set.hotkeys' as const },
    { id: 'folders', icon: IconFolder, key: 'set.folders' as const },
    { id: 'overlay', icon: IconBell, key: 'set.overlay' as const },
    { id: 'appearance', icon: IconPalette, key: 'set.appearance' as const },
    { id: 'system', icon: IconPower, key: 'set.system' as const },
  ];

  $effect(() => {
    // Recompute the estimate whenever anything that affects it changes.
    const e = s.engine;
    void [e.monitor, e.resolution, e.codec, e.quality, e.fps, e.bitrateKbps, e.mic, e.systemAudio, e.separateTracks, s.replaySeconds];
    api.estimate($state.snapshot(s)).then((r) => (estimate = r));
  });

  $effect(() => {
    if (app.savedPulse) {
      showSaved = true;
      const t = setTimeout(() => (showSaved = false), 1400);
      return () => clearTimeout(t);
    }
  });

  $effect(() => {
    const id = location.hash.slice(1);
    if (id) requestAnimationFrame(() => document.getElementById(id)?.scrollIntoView({ behavior: 'smooth' }));
  });

  function jump(id: string) {
    active = id;
    document.getElementById(id)?.scrollIntoView({ behavior: 'smooth', block: 'start' });
  }

  function onscroll(e: Event) {
    const el = e.currentTarget as HTMLElement;
    for (const sec of sections) {
      const node = document.getElementById(sec.id);
      if (node && node.offsetTop - el.scrollTop < 160) active = sec.id;
    }
  }

  async function pickDir(which: 'clipsDir' | 'screenshotsDir') {
    const dir = await openDialog({ directory: true, defaultPath: s[which] });
    if (typeof dir === 'string') app.change((x) => (x[which] = dir), 0);
  }

  async function checkUpdate() {
    checking = true;
    try {
      const u = await api.checkUpdate();
      if (!u) app.notify(app.t('set.upToDate'), 'ok');
    } catch (e) {
      app.notify(String(e), 'error');
    } finally {
      checking = false;
    }
  }

  const monitorName = (m: (typeof snap.monitors)[number]) => `${m.name} · ${m.width}×${m.height}${m.primary ? ` (${app.t('set.primary')})` : ''}`;
  const current = $derived(snap.monitors.find((m) => m.id === s.engine.monitor) ?? snap.monitors.find((m) => m.primary));
  const resOptions = $derived(
    (
      [
        ['native', app.t('set.native')],
        ['p2160', '2160p (4K)'],
        ['p1440', '1440p'],
        ['p1080', '1080p'],
        ['p720', '720p'],
      ] as const
    )
      .filter(([v]) => v === 'native' || !current || Number(v.slice(1)) < current.height)
      .map(([value, label]) => ({ value, label })),
  );
  const minutes = (sec: number) => (sec % 60 ? `${Math.floor(sec / 60)}:${String(sec % 60).padStart(2, '0')}` : `${sec / 60}`) + (app.lang === 'ru' ? ' мин' : ' min');
  const hkRows: { key: keyof Hotkeys; label: 'hk.saveClip' | 'hk.toggleReplay' | 'hk.screenshot' | 'hk.toggleRecording' }[] = [
    { key: 'saveClip', label: 'hk.saveClip' },
    { key: 'toggleReplay', label: 'hk.toggleReplay' },
    { key: 'screenshot', label: 'hk.screenshot' },
    { key: 'toggleRecording', label: 'hk.toggleRecording' },
  ];
</script>

<div class="layout">
  <aside class="subnav">
    <h1>{app.t('set.title')}</h1>
    {#each sections as sec}
      <button class:active={active === sec.id} onclick={() => jump(sec.id)}>
        <sec.icon size={18} />{app.t(sec.key)}
      </button>
    {/each}
    <div class="saved" class:show={showSaved}><IconCheck size={15} />{app.t('set.saved')}</div>
  </aside>

  <div class="scroll" {onscroll}>
    <section id="capture" class="card sec">
      <h2><IconVideo size={20} />{app.t('set.capture')}</h2>
      <Row label={app.t('set.replayEnabled')} hint={app.t('set.replayEnabledHint')}>
        <Switch checked={s.replayEnabled} onchange={(v) => api.setReplay(v)} />
      </Row>
      <Row label={app.t('set.length')} hint={app.t('set.lengthHint')}>
        <Slider value={s.replaySeconds} min={60} max={1200} step={30} format={minutes} width="300px" onchange={(v) => app.change((x) => (x.replaySeconds = v), 600)} />
      </Row>
      <Row label={app.t('set.quality')}>
        <Segmented
          value={s.engine.quality}
          onchange={(v) => app.change((x) => (x.engine.quality = v))}
          options={(['low', 'medium', 'high', 'ultra'] as const).map((q) => ({ value: q, label: app.t(`set.q.${q}`) }))}
        />
      </Row>
      <Row label={app.t('set.resolution')}>
        <Select value={s.engine.resolution} options={resOptions} onchange={(v) => app.change((x) => (x.engine.resolution = v as typeof x.engine.resolution))} />
      </Row>
      <Row label={app.t('set.fps')}>
        <Segmented value={s.engine.fps} onchange={(v) => app.change((x) => (x.engine.fps = v))} options={[30, 60, 120, 144].map((f) => ({ value: f, label: String(f) }))} />
      </Row>
      <Row label={app.t('set.codec')} hint={s.engine.codec === 'av1' ? app.t('set.av1Warn') : app.t('set.codecHint')}>
        <Segmented
          value={s.engine.codec}
          onchange={(v) => app.change((x) => (x.engine.codec = v))}
          options={[
            { value: 'h264', label: 'H.264' },
            { value: 'hevc', label: 'HEVC' },
            { value: 'av1', label: 'AV1' },
          ]}
        />
      </Row>
      {#if snap.monitors.length > 1}
        <Row label={app.t('set.monitor')}>
          <Select
            width="320px"
            value={s.engine.monitor ?? current?.id ?? ''}
            options={snap.monitors.map((m) => ({ value: m.id, label: monitorName(m) }))}
            onchange={(v) => app.change((x) => (x.engine.monitor = v))}
          />
        </Row>
      {/if}
      <Row label={app.t('set.cursor')}>
        <Switch checked={s.engine.captureCursor} onchange={(v) => app.change((x) => (x.engine.captureCursor = v))} />
      </Row>
      {#if estimate}
        <div class="estimate">
          <span class="res">{estimate.width}×{estimate.height} · {s.engine.fps} fps</span>
          {app.t('set.estimate', {
            mbps: (estimate.bitrateKbps / 1000).toFixed(0),
            mem: estimate.bufferMb >= 1024 ? `${(estimate.bufferMb / 1024).toFixed(1).replace('.', app.lang === 'ru' ? ',' : '.')} ${app.lang === 'ru' ? 'ГБ' : 'GB'}` : `${estimate.bufferMb} ${app.lang === 'ru' ? 'МБ' : 'MB'}`,
          })}
        </div>
      {/if}
    </section>

    <section id="audio" class="card sec">
      <h2><IconVolume size={20} />{app.t('set.audio')}</h2>
      <Row label={app.t('set.system_audio')}>
        <Switch checked={s.engine.systemAudio} onchange={(v) => app.change((x) => (x.engine.systemAudio = v))} />
      </Row>
      {#if s.engine.systemAudio}
        <Row label={app.t('set.device')}>
          <Select
            width="320px"
            value={s.engine.systemDevice ?? ''}
            options={[{ value: '', label: app.t('set.default') }, ...snap.audioOutputs.map((d) => ({ value: d.id, label: d.name }))]}
            onchange={(v) => app.change((x) => (x.engine.systemDevice = v || null))}
          />
        </Row>
        <Row label={app.t('set.volume')}>
          <Slider value={Math.round(s.engine.systemVolume * 100)} min={0} max={200} step={5} format={(v) => `${v}%`} onchange={(v) => app.change((x) => (x.engine.systemVolume = v / 100), 600)} />
        </Row>
      {/if}
      <Row label={app.t('set.mic')} hint={app.t('set.micHint')}>
        <Switch checked={s.engine.mic} onchange={(v) => app.change((x) => (x.engine.mic = v))} />
      </Row>
      {#if s.engine.mic}
        <Row label={app.t('set.device')}>
          <Select
            width="320px"
            value={s.engine.micDevice ?? ''}
            options={[{ value: '', label: app.t('set.default') }, ...snap.audioInputs.map((d) => ({ value: d.id, label: d.name }))]}
            onchange={(v) => app.change((x) => (x.engine.micDevice = v || null))}
          />
        </Row>
        <Row label={app.t('set.volume')}>
          <Slider value={Math.round(s.engine.micVolume * 100)} min={0} max={300} step={5} format={(v) => `${v}%`} onchange={(v) => app.change((x) => (x.engine.micVolume = v / 100), 600)} />
        </Row>
      {/if}
      {#if s.engine.mic && s.engine.systemAudio}
        <Row label={app.t('set.separate')} hint={app.t('set.separateHint')}>
          <Switch checked={s.engine.separateTracks} onchange={(v) => app.change((x) => (x.engine.separateTracks = v))} />
        </Row>
      {/if}
    </section>

    <section id="hotkeys" class="card sec">
      <h2><IconKeyboard size={20} />{app.t('set.hotkeys')}</h2>
      {#each hkRows as r}
        <Row label={app.t(r.label)} hint={app.hotkeyErrors.includes(r.key) ? app.t('home.hotkeyConflict') : ''}>
          <HotkeyInput value={s.hotkeys[r.key]} conflict={app.hotkeyErrors.includes(r.key)} onchange={(v) => app.change((x) => (x.hotkeys[r.key] = v), 0)} />
        </Row>
      {/each}
    </section>

    <section id="folders" class="card sec">
      <h2><IconFolder size={20} />{app.t('set.folders')}</h2>
      {#each [['clipsDir', 'set.clipsDir'], ['screenshotsDir', 'set.shotsDir']] as const as [k, label]}
        <Row label={app.t(label)} hint={s[k]}>
          <button class="btn sm" onclick={() => api.openMediaDir(k === 'screenshotsDir')}>{app.t('set.open')}</button>
          <button class="btn sm" onclick={() => pickDir(k)}>{app.t('set.change')}</button>
        </Row>
      {/each}
      <Row label={app.t('set.sortByGame')} hint={app.t('set.sortByGameHint')}>
        <Switch checked={s.sortByGame} onchange={(v) => app.change((x) => (x.sortByGame = v))} />
      </Row>
    </section>

    <section id="overlay" class="card sec">
      <h2><IconBell size={20} />{app.t('set.overlay')}</h2>
      <Row label={app.t('set.overlayEnabled')} hint={app.t('set.overlayHint')}>
        <Switch checked={s.overlay.enabled} onchange={(v) => app.change((x) => (x.overlay.enabled = v))} />
      </Row>
      {#if s.overlay.enabled}
        <Row label={app.t('set.corner')}>
          <div class="corners">
            {#each ['top-left', 'top-right', 'bottom-left', 'bottom-right'] as c}
              <button class="corner {c}" class:active={s.overlay.corner === c} aria-label={c} onclick={() => app.change((x) => (x.overlay.corner = c), 0)}>
                <span></span>
              </button>
            {/each}
          </div>
          <button class="btn sm" onclick={() => invoke('preview_toast')}>▶</button>
        </Row>
      {/if}
      <Row label={app.t('set.sound')}>
        <Switch checked={s.overlay.sound} onchange={(v) => app.change((x) => (x.overlay.sound = v))} />
      </Row>
    </section>

    <section id="appearance" class="card sec">
      <h2><IconPalette size={20} />{app.t('set.appearance')}</h2>
      <Row label={app.t('set.accent')}>
        <div class="swatches">
          {#each Object.entries(ACCENTS) as [id, acc]}
            <button
              class="sw"
              class:active={s.accent === id}
              title={app.lang === 'ru' ? acc.ru : acc.en}
              style:background="linear-gradient(135deg, {acc.a}, {acc.b})"
              onclick={() => app.change((x) => (x.accent = id), 0)}
            ></button>
          {/each}
        </div>
      </Row>
      <Row label={app.t('set.language')}>
        <Segmented
          value={s.language}
          onchange={(v) => app.change((x) => (x.language = v), 0)}
          options={[
            { value: 'auto', label: app.t('set.lang.auto') },
            { value: 'ru', label: 'Русский' },
            { value: 'en', label: 'English' },
          ]}
        />
      </Row>
    </section>

    <section id="system" class="card sec">
      <h2><IconPower size={20} />{app.t('set.system')}</h2>
      <Row label={app.t('set.autostart')} hint={app.t('set.autostartHint')}>
        <Switch checked={s.autostart} onchange={(v) => app.change((x) => (x.autostart = v))} />
      </Row>
      <Row label={app.t('set.autoUpdate')}>
        <Switch checked={s.autoUpdate} onchange={(v) => app.change((x) => (x.autoUpdate = v))} />
      </Row>
      <div class="about">
        <Logo size={44} />
        <div>
          <div class="brand">GeniusClip</div>
          <div class="muted">{app.t('set.version')} {snap.version}</div>
        </div>
        <span class="grow"></span>
        <button class="btn" disabled={checking} onclick={checkUpdate}><IconRefresh size={17} />{app.t('set.checkUpdate')}</button>
      </div>
    </section>
  </div>
</div>

<style>
  .layout {
    display: flex;
    gap: 28px;
    max-width: 1100px;
    margin: 0 auto;
    height: 100%;
  }
  .subnav {
    width: 210px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .subnav h1 {
    font-size: 26px;
    margin-bottom: 16px;
  }
  .subnav button {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 40px;
    padding: 0 12px;
    border-radius: 11px;
    font-weight: 600;
    color: var(--text-2);
    text-align: left;
    transition:
      background 0.15s,
      color 0.15s;
  }
  .subnav button:hover {
    background: var(--hover);
    color: var(--text);
  }
  .subnav button.active {
    background: var(--active);
    color: var(--text);
  }
  .saved {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 14px;
    padding-left: 12px;
    color: var(--ok);
    font-size: 13px;
    font-weight: 600;
    opacity: 0;
    transition: opacity 0.3s;
  }
  .saved.show {
    opacity: 1;
  }
  .scroll {
    flex: 1;
    min-width: 0;
    height: 100%;
    overflow-y: auto;
    padding-right: 10px;
    display: flex;
    flex-direction: column;
    gap: 18px;
    padding-bottom: 40vh;
  }
  .sec {
    padding: 22px 26px 10px;
    scroll-margin-top: 8px;
  }
  h2 {
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: 17px;
    margin-bottom: 6px;
  }
  h2 :global(svg) {
    color: var(--accent-a);
  }
  .estimate {
    margin: 4px 0 14px;
    padding: 12px 14px;
    border-radius: var(--r);
    background: var(--accent-soft);
    font-size: 13px;
    font-weight: 600;
  }
  .estimate .res {
    font-family: var(--font-display);
    font-weight: 500;
    margin-right: 10px;
  }
  .corners {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 8px;
  }
  .corner {
    position: relative;
    width: 58px;
    height: 36px;
    border-radius: 8px;
    background: rgba(13, 10, 23, 0.6);
    border: 1px solid var(--line-2);
  }
  .corner span {
    position: absolute;
    width: 20px;
    height: 7px;
    border-radius: 3px;
    background: var(--text-3);
  }
  .corner.top-left span {
    top: 5px;
    left: 5px;
  }
  .corner.top-right span {
    top: 5px;
    right: 5px;
  }
  .corner.bottom-left span {
    bottom: 5px;
    left: 5px;
  }
  .corner.bottom-right span {
    bottom: 5px;
    right: 5px;
  }
  .corner.active {
    border-color: var(--accent-a);
  }
  .corner.active span {
    background: var(--accent-grad);
  }
  .swatches {
    display: flex;
    gap: 10px;
  }
  .sw {
    width: 32px;
    height: 32px;
    border-radius: 50%;
    border: 2px solid transparent;
    box-shadow: inset 0 0 0 2px rgba(13, 10, 23, 0.9);
    transition: transform 0.15s;
  }
  .sw:hover {
    transform: scale(1.1);
  }
  .sw.active {
    border-color: #f5f0ff;
  }
  .about {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 18px 0 14px;
    border-top: 1px solid var(--line);
  }
  .brand {
    font-family: var(--font-display);
    font-weight: 600;
    font-size: 16px;
  }
  .grow {
    flex: 1;
  }
</style>
