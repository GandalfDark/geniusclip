<script lang="ts">
  import type { Snippet } from 'svelte';
  let {
    value = 0,
    size = 190,
    stroke = 12,
    active = true,
    children,
  }: { value?: number; size?: number; stroke?: number; active?: boolean; children?: Snippet } = $props();
  const id = `rg${Math.random().toString(36).slice(2, 8)}`;
  let r = $derived((size - stroke) / 2);
  let c = $derived(2 * Math.PI * r);
  let v = $derived(Math.max(0, Math.min(1, value)));
</script>

<div class="ring" style:width="{size}px" style:height="{size}px" class:active>
  <svg width={size} height={size} viewBox="0 0 {size} {size}">
    <defs>
      <linearGradient id={id} x1="0" y1="0" x2="1" y2="1">
        <stop offset="0" stop-color="var(--accent-a)" />
        <stop offset="1" stop-color="var(--accent-b)" />
      </linearGradient>
    </defs>
    <circle cx={size / 2} cy={size / 2} {r} fill="none" stroke="rgba(167,139,250,0.12)" stroke-width={stroke} />
    <circle
      class="arc"
      cx={size / 2}
      cy={size / 2}
      {r}
      fill="none"
      stroke="url(#{id})"
      stroke-width={stroke}
      stroke-linecap="round"
      stroke-dasharray={c}
      stroke-dashoffset={c * (1 - v)}
      transform="rotate(-90 {size / 2} {size / 2})"
    />
  </svg>
  <div class="center">{@render children?.()}</div>
</div>

<style>
  .ring {
    position: relative;
    flex-shrink: 0;
  }
  .ring.active::before {
    content: '';
    position: absolute;
    inset: 18%;
    border-radius: 50%;
    background: radial-gradient(circle, color-mix(in srgb, var(--accent-a) 35%, transparent), transparent 70%);
    filter: blur(18px);
    opacity: 0.55;
  }
  .arc {
    transition: stroke-dashoffset 0.9s var(--ease);
  }
  .center {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    text-align: center;
  }
</style>
