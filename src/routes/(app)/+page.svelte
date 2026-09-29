<script lang="ts">
  import Icon from '$lib/components/Icon.svelte';
  import Switch from '$lib/components/Switch.svelte';
  import Keys from '$lib/components/Keys.svelte';
  import MediaCard from '$lib/components/MediaCard.svelte';
  import Viewer from '$lib/components/Viewer.svelte';
  import { onMount } from 'svelte';
  import { flip } from 'svelte/animate';
  import { slide } from 'svelte/transition';
  import { api } from '$lib/api';
  import { enter } from '$lib/enter';
  import { DUR, easeOut, flipParams, leave, reduced, rise } from '$lib/motion';
  import type { Origin } from '$lib/components/Viewer.svelte';
  import { app } from '$lib/app.svelte';
  import { bytes, duration, hotkeyParts, releaseNotes } from '$lib/format';
  import { readyLine, type TKey } from '$lib/i18n';
  import type { DiskSpace, MediaEntry } from '$lib/types';

  let s = $derived(app.settings!);
  let st = $derived(app.status);
  let on = $derived(s.replayEnabled);
  let running = $derived(!!st?.running);
  // Display-off and lock pauses are invisible (the screen is off); only a
  // battery pause can be seen here.
  let paused = $derived(!!st?.paused);
  // "Only in games" with no game: calm, nothing is wrong.
  let waiting = $derived(on && app.waitingForGame);
  let recent = $derived(app.media.filter((m) => m.kind !== 'screenshot').slice(0, 8));
  let viewing = $state<MediaEntry | null>(null);
  let origin = $state<Origin | null>(null);
  let justSaved = $state(false);

  // The main button confirms each saved clip for a moment (hotkey saves too),
  // but not one saved before this page was shown.
  let seenSaved = app.clipSavedAt;
  $effect(() => {
    const at = app.clipSavedAt;
    if (at === seenSaved) return;
    seenSaved = at;
    checkDisk();
    justSaved = true;
    const t = setTimeout(() => (justSaved = false), 1500);
    return () => clearTimeout(t);
  });

  // --- Free space on the clips drive: checked on open, when the window comes
  // back to the front, and after each saved clip.
  let disk = $state<DiskSpace | null>(null);
  let diskSeq = 0;
  async function checkDisk() {
    const seq = ++diskSeq;
    try {
      const d = await api.diskSpace();
      if (seq === diskSeq) disk = d;
    } catch {
      /* keep the last answer */
    }
  }
  onMount(() => {
    checkDisk();
    // Focus: back from a game; visible: shown again from the tray.
    const onVisible = () => document.visibilityState === 'visible' && checkDisk();
    window.addEventListener('focus', checkDisk);
    document.addEventListener('visibilitychange', onVisible);
    return () => {
      window.removeEventListener('focus', checkDisk);
      document.removeEventListener('visibilitychange', onVisible);
    };
  });

  // --- What's new after an update: the release notes fold out of the card.
  let news = $derived(app.snapshot?.whatsNew ?? null);
  let newsBlocks = $derived(news ? releaseNotes(news.notes) : []);
  let newsOpen = $state(false);
  const fold = () => ({ duration: reduced() ? 0 : DUR, easing: easeOut });

  // --- First steps: each one ticks itself (the backend records the first
  // saved clip and the first menu opening).
  let onb = $derived(s.onboarding);
  let steps = $derived([
    { title: 'onb.replay', done: on, accel: null },
    { title: 'onb.save', done: !!onb?.clipSaved, accel: s.hotkeys.saveClip },
    { title: 'onb.menu', done: !!onb?.menuOpened, accel: s.hotkeys.toggleMenu },
  ] satisfies { title: TKey; done: boolean; accel: string | null }[]);
  let doneCount = $derived(steps.filter((x) => x.done).length);
  let allDone = $derived(doneCount === steps.length);
  let nextStep = $derived(steps.findIndex((x) => !x.done));
  const hideSteps = () => app.change((x) => (x.onboarding.dismissed = true), 0);
  // All done: "Done" for a moment, then the card goes away for good.
  $effect(() => {
    if (!allDone || !onb || onb.dismissed) return;
    const t = setTimeout(hideSteps, 2600);
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
      <span class="dot" class:live={on && running && !paused && !waiting} class:wait={on && (!running || paused) && !waiting}></span>
      <!-- Animate only the on/off/paused change; capture start-up is shown by the dot. -->
      {#key `${on}-${paused}-${waiting}`}
        <div class="txt" in:rise={{ y: 6 }}>
          {#if waiting}
            <h1>{app.t('home.waitGame')}</h1>
            <p>{app.t('home.waitGameHint')}</p>
          {:else if on && paused}
            <h1>{app.t('home.paused')}</h1>
            <p>{app.t('home.pausedBattery')}</p>
          {:else}
            <h1>{on ? app.t('home.on') : app.t('home.off')}</h1>
            <p>{on ? readyLine(app.lang, s.replaySeconds) : app.t('home.offHint')}</p>
          {/if}
        </div>
      {/key}
      {#if on}
        <Switch checked={on} label={app.t('set.replayEnabled')} onchange={(v) => api.setReplay(v)} />
      {:else}
        <button class="btn primary" onclick={() => api.setReplay(true)}>{app.t('home.turnOn')}</button>
      {/if}
    </div>

    {#if st?.lastError && on}
      <div class="err"><Icon name="alert" size={16} />{app.t('home.error')}: {app.errorText(st.lastError)}</div>
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

  <!-- Warnings first (saving is at risk), then news. -->
  <div class="notes">
    {#if disk?.low}
      <div class="note panel warnnote" in:rise={{ y: 6 }} out:leave>
        <Icon name="alert" size={18} />
        <span>{app.t('home.lowDisk', { drive: disk.drive, free: bytes(disk.freeMb * 1048576, app.lang) })}</span>
        <span class="grow"></span>
        <button class="btn sm" onclick={() => api.openMediaDir(false)}>{app.t('home.openFolder')}</button>
      </div>
    {/if}
    {#if app.hotkeyErrors.length}
      <a class="note panel warnnote" href="/settings#hotkeys">
        <Icon name="alert" size={18} /><span>{app.t('home.hotkeyConflict')}</span>
      </a>
    {/if}
    {#if app.update}
      <div class="note panel">
        <Icon name="update" size={18} />
        <span>{app.t('home.update', { v: app.update.version })}</span>
        <span class="grow"></span>
        <button class="btn primary sm" disabled={updating} onclick={install}>{app.t('home.updateBtn')}</button>
      </div>
    {/if}
    {#if news}
      <div class="note panel news" out:leave>
        <div class="news-row">
          <Icon name="sparkle" size={18} />
          <span>{app.t('news.title', { v: news.version })}</span>
          <span class="grow"></span>
          {#if newsBlocks.length}
            <button class="btn ghost sm" aria-expanded={newsOpen} onclick={() => (newsOpen = !newsOpen)}>
              {app.t('news.show')}<span class="chev" class:open={newsOpen}><Icon name="down" size={15} /></span>
            </button>
          {/if}
          <button class="btn ghost sm icon" title={app.t('home.hide')} aria-label={app.t('home.hide')} onclick={() => app.dismissWhatsNew()}>
            <Icon name="close" size={16} />
          </button>
        </div>
        {#if newsOpen}
          <div class="news-body" transition:slide={fold()}>
            {#each newsBlocks as b}
              {#if 'items' in b}
                <ul>
                  {#each b.items as item}<li>{item}</li>{/each}
                </ul>
              {:else}
                <p>{b.text}</p>
              {/if}
            {/each}
          </div>
        {/if}
      </div>
    {/if}
  </div>

  {#if onb && !onb.dismissed}
    <section class="onb panel" class:complete={allDone} out:leave aria-labelledby="onb-title">
      <div class="onb-head">
        {#key allDone}
          <h2 id="onb-title" in:rise={{ y: 4, duration: 260 }}>
            {#if allDone}<Icon name="check" size={16} stroke={2.2} />{app.t('onb.done')}{:else}{app.t('onb.title', { n: doneCount })}{/if}
          </h2>
        {/key}
        <span class="grow"></span>
        {#if !allDone}<button class="hide" onclick={hideSteps}>{app.t('home.hide')}</button>{/if}
      </div>
      <ol class="steps">
        {#each steps as step, i}
          <li class="step" class:done={step.done} class:next={i === nextStep}>
            <span class="num mono">
              {#if step.done}<Icon name="check" size={13} stroke={2.6} />{:else}{i + 1}{/if}
            </span>
            <div class="step-txt">
              <div class="step-title">{app.t(step.title)}</div>
              <div class="step-hint">
                {#if step.accel === null}
                  {app.t('onb.replayHint')}
                {:else if step.accel}
                  {@const parts = app.t('onb.press').split('{key}')}
                  {parts[0]}<Keys accel={step.accel} />{parts[1] ?? ''}
                {:else}
                  <a href="/settings#hotkeys">{app.t('onb.noKey')}</a>
                {/if}
              </div>
            </div>
          </li>
        {/each}
      </ol>
    </section>
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
      <p class="muted empty">
        {s.hotkeys.saveClip
          ? app.t('home.empty', { key: hotkeyParts(s.hotkeys.saveClip).join(' + '), min: Math.round(s.replaySeconds / 60) })
          : app.t('home.emptyNoKey', { min: Math.round(s.replaySeconds / 60) })}
      </p>
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
    position: relative;
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
  }
  /* The pulse ring only changes transform and opacity, which the compositor
     animates without repainting the page every frame (a box-shadow pulse
     did, for as long as Home was open). */
  .dot.live::after {
    content: '';
    position: absolute;
    inset: -5px;
    border-radius: 50%;
    background: color-mix(in srgb, var(--rec) 45%, transparent);
    animation: pulse 2.2s ease-out infinite;
    will-change: transform, opacity;
    pointer-events: none;
  }
  .dot.wait {
    background: var(--warn);
  }
  .dot {
    transition: background var(--dur) var(--ease);
  }
  @keyframes pulse {
    0% {
      transform: scale(0.5);
      opacity: 0.9;
    }
    70%,
    100% {
      transform: scale(1.5);
      opacity: 0;
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
    background: color-mix(in srgb, var(--warn) 6%, var(--panel));
    border-color: color-mix(in srgb, var(--warn) 20%, var(--line));
  }
  .notes {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  /* No notes: no gap for it in the page either. */
  .notes:not(:has(> *)) {
    display: none;
  }
  .news {
    flex-direction: column;
    align-items: stretch;
    gap: 0;
    padding: 6px 8px 6px 14px;
  }
  .news-row {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .news-row > :global(.ic-sparkle) {
    color: var(--accent);
  }
  .chev {
    display: inline-flex;
    transition: transform var(--dur) var(--ease);
  }
  .chev.open {
    transform: rotate(180deg);
  }
  .news-body {
    padding: 4px 0 8px 28px;
    color: var(--text-2);
    font-size: 13px;
    max-height: 260px;
    overflow-y: auto;
    user-select: text;
  }
  .news-body p {
    margin: 6px 0 2px;
    color: var(--text);
    font-weight: 500;
  }
  .news-body ul {
    margin: 2px 0;
    padding-left: 18px;
  }
  .news-body li {
    margin: 3px 0;
  }
  .news-body li::marker {
    color: var(--accent);
  }
  .onb {
    padding: 14px 18px 18px;
  }
  .onb-head {
    display: flex;
    align-items: center;
    min-height: 26px;
    margin-bottom: 14px;
  }
  .onb-head h2 {
    display: inline-flex;
    align-items: center;
    gap: 7px;
  }
  .onb.complete .onb-head h2 {
    color: var(--accent);
  }
  .hide {
    font-size: 12.5px;
    color: var(--text-3);
  }
  .hide:hover {
    color: var(--text);
  }
  .steps {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
    gap: 10px;
  }
  .step {
    display: flex;
    align-items: flex-start;
    gap: 11px;
    padding: 12px 14px;
    border-radius: var(--r);
    background: var(--panel-2);
    border: 1px solid var(--line);
    transition:
      border-color var(--dur) var(--ease),
      background var(--dur) var(--ease);
  }
  .step.next {
    border-color: color-mix(in srgb, var(--accent) 45%, var(--line));
  }
  .step.done {
    background: transparent;
  }
  .num {
    flex-shrink: 0;
    width: 22px;
    height: 22px;
    display: grid;
    place-items: center;
    border-radius: 50%;
    border: 1px solid var(--line-2);
    font-size: 11px;
    color: var(--text-2);
    transition:
      background var(--dur) var(--ease),
      border-color var(--dur) var(--ease);
  }
  .step.next .num {
    border-color: var(--accent);
    color: var(--accent);
  }
  .step.done .num {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--accent-ink);
  }
  .step-txt {
    min-width: 0;
  }
  .step-title {
    font-size: 13.5px;
    font-weight: 500;
    line-height: 22px;
  }
  .step-hint {
    margin-top: 2px;
    font-size: 12.5px;
    color: var(--text-2);
    line-height: 1.7;
  }
  .step-hint :global(.keys) {
    vertical-align: 1px;
    margin: 0 2px;
  }
  .step-hint a {
    color: var(--accent);
    text-decoration: none;
  }
  .step-hint a:hover {
    text-decoration: underline;
  }
  .step.done :is(.step-title, .step-hint) {
    color: var(--text-3);
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
