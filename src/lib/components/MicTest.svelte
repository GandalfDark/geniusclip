<script lang="ts">
  // "Listen to myself": the microphone played back live through the current
  // noise suppression, with level meters before and after it.
  import { onDestroy, untrack } from 'svelte';
  import { slide } from 'svelte/transition';
  import { listen } from '@tauri-apps/api/event';
  import Icon from './Icon.svelte';
  import Row from './Row.svelte';
  import { api } from '$lib/api';
  import { app } from '$lib/app.svelte';
  import { DUR_FAST } from '$lib/motion';

  let { device, volume }: { device: string | null; volume: number } = $props();

  let on = $state(false);
  let before = $state(0);
  let after = $state(0);

  // Meters use a dB scale (-60..0 dBFS) with a short falloff.
  const level = (peak: number) => Math.max(0, Math.min(1, (20 * Math.log10(Math.max(peak, 1e-6)) + 60) / 60));

  async function start() {
    try {
      await api.micTest(true);
      on = true;
    } catch (e) {
      app.notify(String(e), 'error');
    }
  }

  function stop() {
    on = false;
    before = after = 0;
    api.micTest(false).catch(() => {});
  }

  $effect(() => {
    const offs = [
      listen<[number, number]>('mic://level', (e) => {
        if (!on) return;
        before = Math.max(level(e.payload[0]), before * 0.8);
        after = Math.max(level(e.payload[1]), after * 0.8);
      }),
      listen<string | null>('mic://stopped', (e) => {
        on = false;
        before = after = 0;
        if (e.payload) app.notify(`${app.t('set.micTestFailed')}: ${e.payload}`, 'error');
      }),
    ];
    return () => offs.forEach((p) => p.then((off) => off()));
  });

  // Picking another microphone (or volume) while listening restarts the
  // check once the new settings have reached the engine.
  $effect(() => {
    void [device, volume];
    if (!untrack(() => on)) return;
    const t = setTimeout(() => untrack(() => on) && api.micTest(true).catch(() => {}), 800);
    return () => clearTimeout(t);
  });

  onDestroy(() => {
    if (on) api.micTest(false).catch(() => {});
  });
</script>

<Row label={app.t('set.micTest')} hint={app.t('set.micTestHint')}>
  <button class="btn" class:listening={on} onclick={on ? stop : start}>
    <Icon name={on ? 'stop' : 'headphones'} size={16} />{on ? app.t('set.micTestStop') : app.t('set.micTestStart')}
  </button>
</Row>
{#if on}
  <div class="meters" transition:slide={{ duration: DUR_FAST * 2 }}>
    <span class="name">{app.t('set.meterBefore')}</span>
    <div class="meter"><i class="raw" style:transform="scaleX({before})"></i></div>
    <span class="name">{app.t('set.meterAfter')}</span>
    <div class="meter"><i class="clean" style:transform="scaleX({after})"></i></div>
  </div>
{/if}

<style>
  .btn.listening {
    border-color: var(--accent);
    color: var(--accent);
  }
  .meters {
    display: grid;
    grid-template-columns: auto 1fr;
    align-items: center;
    gap: 8px 14px;
    padding: 4px 0 14px;
  }
  .name {
    font-family: var(--mono);
    font-size: 11px;
    color: var(--text-3);
  }
  .meter {
    height: 6px;
    border-radius: 3px;
    background: var(--line-2);
    overflow: hidden;
  }
  .meter i {
    display: block;
    height: 100%;
    transform-origin: left;
    transition: transform 80ms linear;
  }
  .raw {
    background: #5b5b66;
  }
  .clean {
    background: var(--accent-grad);
  }
</style>
