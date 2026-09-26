<script lang="ts">
  import { open as openDialog } from '@tauri-apps/plugin-dialog';
  import { invoke } from '@tauri-apps/api/core';
  import Icon from '$lib/components/Icon.svelte';
  import Row from '$lib/components/Row.svelte';
  import Switch from '$lib/components/Switch.svelte';
  import Segmented from '$lib/components/Segmented.svelte';
  import Select from '$lib/components/Select.svelte';
  import Slider from '$lib/components/Slider.svelte';
  import HotkeyInput from '$lib/components/HotkeyInput.svelte';
  import MicTest from '$lib/components/MicTest.svelte';
  import Logo from '$lib/components/Logo.svelte';
  import { api } from '$lib/api';
  import { cascade, rise } from '$lib/motion';
  import { app } from '$lib/app.svelte';
  import { ACCENTS } from '$lib/accents';
  import { LANGS, LOCALES } from '$lib/i18n';
  import { bytes } from '$lib/format';
  import type { Estimate, Hotkeys } from '$lib/types';
  import type { TKey } from '$lib/i18n';

  let s = $derived(app.settings!);
  let snap = $derived(app.snapshot!);
  let estimate = $state<Estimate | null>(null);
  let checking = $state(false);
  let active = $state('capture');
  let showSaved = $state(false);
  let scroller: HTMLElement;

  const sections = [
    { id: 'capture', key: 'set.capture' },
    { id: 'audio', key: 'set.audio' },
    { id: 'hotkeys', key: 'set.hotkeys' },
    { id: 'folders', key: 'set.folders' },
    { id: 'overlay', key: 'set.overlay' },
    { id: 'appearance', key: 'set.appearance' },
    { id: 'system', key: 'set.system' },
  ] as const;

  $effect(() => {
    const e = s.engine;
    void [e.monitor, e.resolution, e.codec, e.quality, e.fps, e.bitrateKbps, e.mic, e.systemAudio, e.separateTracks, e.diskBuffer, s.replaySeconds];
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
    if (id) requestAnimationFrame(() => jump(id));
  });

  function jump(id: string) {
    active = id;
    const node = document.getElementById(id);
    if (node && scroller) scroller.scrollTo({ top: node.offsetTop - scroller.offsetTop, behavior: 'smooth' });
  }

  function onscroll() {
    for (const sec of sections) {
      const node = document.getElementById(sec.id);
      if (node && node.offsetTop - scroller.offsetTop - scroller.scrollTop < 120) active = sec.id;
    }
  }

  async function pickDir(which: 'clipsDir' | 'screenshotsDir') {
    const dir = await openDialog({ directory: true, defaultPath: s[which] });
    if (typeof dir === 'string') app.change((x) => (x[which] = dir), 0);
  }

  // A found update shows up right here (and on Home).
  let updating = $state(false);
  async function installUpdate() {
    updating = true;
    try {
      await api.installUpdate();
    } catch (e) {
      app.notify(String(e), 'error');
      updating = false;
    }
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

  const current = $derived(snap.monitors.find((m) => m.id === s.engine.monitor) ?? snap.monitors.find((m) => m.primary));
  const resOptions = $derived(
    (
      [
        ['native', app.t('set.native')],
        ['p2160', '2160p'],
        ['p1440', '1440p'],
        ['p1080', '1080p'],
        ['p720', '720p'],
      ] as const
    )
      .filter(([v]) => v === 'native' || !current || Number(v.slice(1)) < current.height)
      .map(([value, label]) => ({ value, label })),
  );
  // More than a quarter of the PC's memory for the RAM buffer is a lot.
  let memHeavy = $derived(!!estimate && !s.engine.diskBuffer && snap.ramTotalMb > 0 && estimate.bufferMb > snap.ramTotalMb / 4);
  let micIsBluetooth = $derived(
    !!(s.engine.micDevice ? snap.audioInputs.find((d) => d.id === s.engine.micDevice) : snap.audioInputs.find((d) => d.isDefault))?.bluetooth,
  );
  const MAX_RAM = 1200;
  const MAX_DISK = 3600;
  const minutes = (sec: number) => `${Math.floor(sec / 60)}:${String(sec % 60).padStart(2, '0')}`;
  const memLabel = (mb: number) => bytes(mb * 1048576, app.lang);
  const hkRows: { key: keyof Hotkeys; label: 'hk.saveClip' | 'hk.toggleReplay' | 'hk.screenshot' | 'hk.toggleRecording' | 'hk.saveShort' | 'hk.toggleMenu' }[] = [
    { key: 'saveClip', label: 'hk.saveClip' },
    { key: 'saveShort', label: 'hk.saveShort' },
    { key: 'toggleReplay', label: 'hk.toggleReplay' },
    { key: 'screenshot', label: 'hk.screenshot' },
    { key: 'toggleRecording', label: 'hk.toggleRecording' },
    { key: 'toggleMenu', label: 'hk.toggleMenu' },
  ];
  const shortOptions = [10, 15, 30, 60];
</script>

<div class="layout">
  <aside class="subnav">
    {#each sections as sec}
      <button class:active={active === sec.id} onclick={() => jump(sec.id)}>{app.t(sec.key)}</button>
    {/each}
    <div class="saved mono" class:show={showSaved}>{#key app.savedPulse}<Icon name="check" size={14} stroke={2} />{/key}{app.t('set.saved')}</div>
  </aside>

  <div class="scroll" bind:this={scroller} {onscroll}>
    <section id="capture" in:rise|global={{ delay: cascade(0) }}>
      <h2>{app.t('set.capture')}</h2>
      <div class="panel body">
        <Row label={app.t('set.replayEnabled')} hint={app.t('set.replayEnabledHint')}>
          <Switch checked={s.replayEnabled} onchange={(v) => api.setReplay(v)} />
        </Row>
        <Row label={app.t('set.length')}>
          <Slider
            value={s.replaySeconds}
            min={60}
            max={s.engine.diskBuffer ? MAX_DISK : MAX_RAM}
            step={30}
            format={minutes}
            width="280px"
            onchange={(v) => app.change((x) => (x.replaySeconds = v), 600)}
          />
        </Row>
        <Row label={app.t('set.skipSaved')} hint={app.t('set.skipSavedHint')}>
          <Switch checked={s.skipSaved} onchange={(v) => app.change((x) => (x.skipSaved = v))} />
        </Row>
        <Row label={app.t('set.diskBuffer')} hint={app.t('set.diskBufferHint')}>
          <Switch
            checked={s.engine.diskBuffer}
            onchange={(v) =>
              app.change((x) => {
                x.engine.diskBuffer = v;
                // Long buffers only fit on disk.
                if (!v) x.replaySeconds = Math.min(x.replaySeconds, MAX_RAM);
              })}
          />
        </Row>
        {#if snap.hasBattery}
          <Row label={app.t('set.pauseOnBattery')} hint={app.t('set.pauseOnBatteryHint')}>
            <Switch checked={s.pauseOnBattery} onchange={(v) => app.change((x) => (x.pauseOnBattery = v), 0)} />
          </Row>
        {/if}
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
          <Segmented mono value={s.engine.fps} onchange={(v) => app.change((x) => (x.engine.fps = v))} options={[30, 60, 120, 144].map((f) => ({ value: f, label: String(f) }))} />
        </Row>
        <Row label={app.t('set.codec')} hint={s.engine.codec === 'av1' ? app.t('set.av1Warn') : app.t('set.codecHint')}>
          <Segmented
            mono
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
              width="300px"
              value={s.engine.monitor ?? current?.id ?? ''}
              options={snap.monitors.map((m) => ({ value: m.id, label: `${m.name} · ${m.width}×${m.height}${m.primary ? ` · ${app.t('set.primary')}` : ''}` }))}
              onchange={(v) => app.change((x) => (x.engine.monitor = v))}
            />
          </Row>
        {/if}
        <Row label={app.t('set.cursor')}>
          <Switch checked={s.engine.captureCursor} onchange={(v) => app.change((x) => (x.engine.captureCursor = v))} />
        </Row>
        {#if estimate}
          <div class="estimate mono">
            {estimate.width}×{estimate.height} · {s.engine.fps} {app.t('home.fps')} · {app.t(s.engine.diskBuffer ? 'set.estimateDisk' : 'set.estimate', {
              mbps: (estimate.bitrateKbps / 1000).toFixed(0),
              mem: memLabel(estimate.bufferMb),
            })}
          </div>
          {#if memHeavy}
            <div class="memwarn">
              <Icon name="alert" size={16} />
              <span>{app.t('set.memWarn', { mem: memLabel(estimate.bufferMb), total: memLabel(snap.ramTotalMb) })}</span>
              <button class="btn sm" onclick={() => app.change((x) => (x.engine.diskBuffer = true), 0)}>{app.t('set.memWarnFix')}</button>
            </div>
          {/if}
        {/if}
      </div>
    </section>

    <section id="audio" in:rise|global={{ delay: cascade(1) }}>
      <h2>{app.t('set.audio')}</h2>
      <div class="panel body">
        <Row label={app.t('set.system_audio')}>
          <Switch checked={s.engine.systemAudio} onchange={(v) => app.change((x) => (x.engine.systemAudio = v))} />
        </Row>
        {#if s.engine.systemAudio}
          <Row label={app.t('set.device')}>
            <Select
              width="300px"
              value={s.engine.systemDevice ?? ''}
              options={[{ value: '', label: app.t('set.default') }, ...snap.audioOutputs.map((d) => ({ value: d.id, label: d.name }))]}
              onchange={(v) => app.change((x) => (x.engine.systemDevice = v || null))}
            />
          </Row>
          <Row label={app.t('set.volume')}>
            <Slider value={Math.round(s.engine.systemVolume * 100)} min={0} max={200} step={5} format={(v) => `${v}%`} onchange={(v) => app.change((x) => (x.engine.systemVolume = v / 100), 600)} />
          </Row>
        {/if}
        <Row label={app.t('set.mic')}>
          <Switch checked={s.engine.mic} onchange={(v) => app.change((x) => (x.engine.mic = v))} />
        </Row>
        {#if s.engine.mic}
          <Row label={app.t('set.device')}>
            <Select
              width="300px"
              value={s.engine.micDevice ?? ''}
              options={[{ value: '', label: app.t('set.default') }, ...snap.audioInputs.map((d) => ({ value: d.id, label: d.name }))]}
              onchange={(v) => app.change((x) => (x.engine.micDevice = v || null))}
            />
          </Row>
          {#if micIsBluetooth}
            <div class="memwarn"><Icon name="alert" size={16} /><span>{app.t('set.btMic')}</span></div>
          {/if}
          <Row label={app.t('set.volume')}>
            <Slider value={Math.round(s.engine.micVolume * 100)} min={0} max={300} step={5} format={(v) => `${v}%`} onchange={(v) => app.change((x) => (x.engine.micVolume = v / 100), 600)} />
          </Row>
          <Row label={app.t('set.noise')} hint={app.status?.noiseUnavailable ? app.t('set.noiseFailed') : app.t('set.noiseHint')}>
            <Switch checked={s.engine.noiseSuppression} onchange={(v) => app.change((x) => (x.engine.noiseSuppression = v), 0)} />
          </Row>
          {#if s.engine.noiseSuppression}
            <Row label={app.t('set.noiseStrength')}>
              <Slider value={s.engine.noiseStrength} min={0} max={100} step={5} format={(v) => `${v}%`} onchange={(v) => app.change((x) => (x.engine.noiseStrength = v), 120)} />
            </Row>
          {/if}
          <MicTest device={s.engine.micDevice} volume={s.engine.micVolume} />
        {/if}
        {#if s.engine.mic && s.engine.systemAudio}
          <Row label={app.t('set.separate')} hint={app.t('set.separateHint')}>
            <Switch checked={s.engine.separateTracks} onchange={(v) => app.change((x) => (x.engine.separateTracks = v))} />
          </Row>
        {/if}
      </div>
    </section>

    <section id="hotkeys" in:rise|global={{ delay: cascade(2) }}>
      <h2>{app.t('set.hotkeys')}</h2>
      <div class="panel body">
        {#each hkRows as r}
          <Row
            label={app.t(r.label)}
            hint={app.hotkeyErrors.includes(r.key) ? app.t('home.hotkeyConflict') : r.key === 'saveShort' && !s.hotkeys.saveShort ? app.t('hk.saveShortHint') : ''}
          >
            <HotkeyInput value={s.hotkeys[r.key]} conflict={app.hotkeyErrors.includes(r.key)} onchange={(v) => app.change((x) => (x.hotkeys[r.key] = v), 0)} />
          </Row>
          {#if r.key === 'saveShort' && s.hotkeys.saveShort}
            <Row label={app.t('set.shortLength')}>
              <Segmented
                mono
                value={s.shortSeconds}
                onchange={(v) => app.change((x) => (x.shortSeconds = v))}
                options={shortOptions.map((n) => ({ value: n, label: `${n} ${app.t('set.sec')}` }))}
              />
            </Row>
          {/if}
        {/each}
      </div>
    </section>

    <section id="folders" in:rise|global={{ delay: cascade(3) }}>
      <h2>{app.t('set.folders')}</h2>
      <div class="panel body">
        {#each [['clipsDir', 'set.clipsDir'], ['screenshotsDir', 'set.shotsDir']] as const as [k, label]}
          <Row label={app.t(label)} hint={s[k]}>
            <button class="btn sm" onclick={() => api.openMediaDir(k === 'screenshotsDir')}>{app.t('set.open')}</button>
            <button class="btn sm" onclick={() => pickDir(k)}>{app.t('set.change')}</button>
          </Row>
        {/each}
        <Row label={app.t('set.sortByGame')}>
          <Switch checked={s.sortByGame} onchange={(v) => app.change((x) => (x.sortByGame = v))} />
        </Row>
      </div>
    </section>

    <section id="overlay" in:rise|global={{ delay: cascade(4) }}>
      <h2>{app.t('set.overlay')}</h2>
      <div class="panel body">
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
            <button class="btn sm" onclick={() => invoke('preview_toast')}>{app.t('set.preview')}</button>
          </Row>
          <Row label={app.t('set.sound')}>
            <Switch checked={s.overlay.sound} onchange={(v) => app.change((x) => (x.overlay.sound = v))} />
          </Row>
        {/if}
      </div>
    </section>

    <section id="appearance" in:rise|global={{ delay: cascade(5) }}>
      <h2>{app.t('set.appearance')}</h2>
      <div class="panel body">
        <Row label={app.t('set.accent')}>
          <div class="swatches">
            {#each Object.entries(ACCENTS) as [id, acc]}
              <button
                class="sw"
                class:active={s.accent === id}
                title={app.t(`accent.${id}` as TKey)}
                style:--c={acc.color}
                onclick={() => app.change((x) => (x.accent = id), 0)}
              ></button>
            {/each}
          </div>
        </Row>
        <Row label={app.t('set.language')}>
          <Select
            width="220px"
            value={s.language}
            onchange={(v) => app.change((x) => (x.language = v), 0)}
            options={[{ value: 'auto', label: app.t('set.lang.auto') }, ...LANGS.map((l) => ({ value: l, label: LOCALES[l].name }))]}
          />
        </Row>
      </div>
    </section>

    <section id="system" in:rise|global={{ delay: cascade(6) }}>
      <h2>{app.t('set.system')}</h2>
      <div class="panel body">
        <Row label={app.t('set.autostart')}>
          <Switch checked={s.autostart} onchange={(v) => app.change((x) => (x.autostart = v))} />
        </Row>
        <Row label={app.t('set.autoUpdate')}>
          <Switch checked={s.autoUpdate} onchange={(v) => app.change((x) => (x.autoUpdate = v))} />
        </Row>
        <div class="about">
          <Logo size={28} />
          <span class="brand">GeniusClip</span>
          <span class="mono faint">{snap.version}</span>
          <span class="grow"></span>
          {#if app.update}
            <span class="upd">{app.t('home.update', { v: app.update.version })}</span>
            <button class="btn primary sm" disabled={updating} onclick={installUpdate}>{app.t('home.updateBtn')}</button>
          {:else}
            <button class="btn sm" disabled={checking} onclick={checkUpdate}>{app.t('set.checkUpdate')}</button>
          {/if}
        </div>
      </div>
    </section>
  </div>
</div>

<style>
  .layout {
    display: flex;
    gap: 36px;
    max-width: 1020px;
    margin: 0 auto;
    height: 100%;
  }
  .subnav {
    width: 150px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    padding-top: 30px;
  }
  .subnav button {
    position: relative;
    height: 32px;
    padding-left: 14px;
    text-align: left;
    font-size: 13.5px;
    color: var(--text-3);
    border-left: 1px solid var(--line-2);
  }
  .subnav button:hover {
    color: var(--text);
  }
  .subnav button.active {
    color: var(--text);
    border-left: 2px solid var(--accent);
    padding-left: 13px;
  }
  .saved {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 16px;
    padding-left: 14px;
    color: var(--accent);
    font-size: 11.5px;
    opacity: 0;
    transition: opacity 0.2s;
  }
  .saved.show {
    opacity: 1;
  }
  .scroll {
    flex: 1;
    min-width: 0;
    height: 100%;
    overflow-y: auto;
    padding: 0 8px 50vh 0;
  }
  section {
    margin-bottom: 26px;
  }
  h2 {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-2);
    margin: 6px 0 8px 2px;
  }
  .body {
    padding: 2px 18px;
  }
  .memwarn {
    display: flex;
    align-items: center;
    gap: 10px;
    margin: 0 -18px;
    padding: 10px 18px;
    border-top: 1px solid var(--line);
    font-size: 12.5px;
    color: var(--warn);
    background: color-mix(in srgb, var(--warn) 6%, transparent);
  }
  .memwarn span {
    flex: 1;
  }
  .memwarn :global(.btn) {
    flex-shrink: 0;
  }
  .upd {
    font-size: 13px;
    color: var(--accent);
  }
  .estimate {
    margin: 0 -18px;
    padding: 10px 18px;
    border-top: 1px solid var(--line);
    font-size: 11.5px;
    color: var(--text-2);
  }
  .corners {
    display: flex;
    gap: 6px;
  }
  .corner {
    position: relative;
    width: 44px;
    height: 28px;
    border-radius: var(--r-sm);
    background: var(--bg);
    border: 1px solid var(--line-2);
  }
  .corner span {
    position: absolute;
    width: 14px;
    height: 5px;
    border-radius: 1px;
    background: var(--text-3);
  }
  .corner.top-left span {
    top: 4px;
    left: 4px;
  }
  .corner.top-right span {
    top: 4px;
    right: 4px;
  }
  .corner.bottom-left span {
    bottom: 4px;
    left: 4px;
  }
  .corner.bottom-right span {
    bottom: 4px;
    right: 4px;
  }
  .corner.active {
    border-color: var(--accent);
  }
  .corner.active span {
    background: var(--accent);
  }
  .swatches {
    display: flex;
    gap: 8px;
  }
  .sw {
    width: 22px;
    height: 22px;
    border-radius: 50%;
    background: var(--c);
    box-shadow:
      0 0 0 2px var(--panel),
      0 0 0 3px transparent;
  }
  .sw.active {
    box-shadow:
      0 0 0 2px var(--panel),
      0 0 0 3px var(--text);
  }
  .about {
    display: flex;
    align-items: center;
    gap: 10px;
    margin: 0 -18px;
    padding: 12px 18px;
    border-top: 1px solid var(--line);
  }
  .brand {
    font-weight: 600;
  }
  .grow {
    flex: 1;
  }
</style>
