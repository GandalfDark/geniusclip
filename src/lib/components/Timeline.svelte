<script lang="ts">
  import { duration } from '$lib/format';
  import { app } from '$lib/app.svelte';

  /** Replay buffer as a DVR timeline: "now" on the right, history grows to the left. */
  let { seconds = 0, total = 300, live = true }: { seconds?: number; total?: number; live?: boolean } = $props();

  let step = $derived(total <= 120 ? 15 : total <= 600 ? 60 : total <= 1200 ? 120 : 300);
  let ticks = $derived(Array.from({ length: Math.floor(total / step) + 1 }, (_, i) => i * step).filter((t) => t <= total));
  let filled = $derived(Math.min(1, Math.max(0, seconds / total)));
</script>

<div class="tl" class:live>
  <div class="track">
    {#each ticks as t}
      <span class="tick" style:left="{100 - (t / total) * 100}%"></span>
    {/each}
    <div class="fill" style:width="{filled * 100}%"></div>
    <span class="head"></span>
  </div>
  <div class="labels mono">
    {#each ticks as t}
      <span class="lbl" class:edge={t === 0} style:left="{100 - (t / total) * 100}%">{t === 0 ? app.t('home.now') : `−${duration(t)}`}</span>
    {/each}
  </div>
</div>

<style>
  .tl {
    width: 100%;
  }
  .track {
    position: relative;
    height: 22px;
    background: repeating-linear-gradient(90deg, transparent 0 5px, #1d1d22 5px 6px), var(--bg);
    border: 1px solid var(--line-2);
    border-radius: var(--r-sm);
    overflow: hidden;
  }
  .tick {
    position: absolute;
    top: 0;
    width: 1px;
    height: 5px;
    background: #4a4a52;
  }
  .fill {
    position: absolute;
    right: 0;
    top: 0;
    bottom: 0;
    background: color-mix(in srgb, var(--accent) 80%, transparent);
    transition: width 0.9s linear;
  }
  .head {
    position: absolute;
    right: 0;
    top: 0;
    bottom: 0;
    width: 2px;
    background: #fff;
  }
  .live .head {
    background: var(--rec);
  }
  .labels {
    position: relative;
    height: 16px;
    margin-top: 5px;
  }
  .lbl {
    position: absolute;
    transform: translateX(-50%);
    font-size: 10.5px;
    color: var(--text-3);
    white-space: nowrap;
  }
  .lbl:first-child {
    transform: none;
  }
  .lbl.edge {
    transform: translateX(-100%);
    color: var(--text-2);
  }
</style>
