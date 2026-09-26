<script lang="ts" module>
  // GeniusClip's own icon set: 24px grid, round caps, one stroke weight.
  // Moving parts are wrapped in <g class="m…"> so hover animations (below)
  // can target them; filled parts use currentColor, the record dot --rec.
  const ICONS = {
    replay: '<g class="m1"><path d="M4.5 12a7.5 7.5 0 1 0 2.2-5.3"/><path d="M4.5 4.5v3.2h3.2"/></g><circle cx="12" cy="12" r="1.7" fill="currentColor" stroke="none"/>',
    film: '<rect x="4" y="5" width="16" height="14" rx="2"/><g class="m1"><path d="M8.5 5v14M15.5 5v14M4 9.5h4.5M4 14.5h4.5M15.5 9.5H20M15.5 14.5H20"/></g>',
    sliders: '<path d="M4 7.5h16M4 16.5h16" stroke-opacity=".55"/><g class="m1"><circle cx="15.5" cy="7.5" r="2.4" fill="currentColor"/></g><g class="m2"><circle cx="8.5" cy="16.5" r="2.4" fill="currentColor"/></g>',
    clapper: '<path d="M4.5 10.5h15v8a1.5 1.5 0 0 1-1.5 1.5H6a1.5 1.5 0 0 1-1.5-1.5z"/><g class="m1"><path d="M4.4 10.3l-.8-2.6a1 1 0 0 1 .7-1.25l12.4-3.5a1 1 0 0 1 1.23.7l.77 2.6z"/><path d="M8.3 5.6l2.4 3.1M12.6 4.4l2.4 3.1"/></g>',
    shot: '<g class="m1"><path d="M4 8.5V5h3.5M16.5 5H20v3.5M20 15.5V19h-3.5M7.5 19H4v-3.5"/></g><circle cx="12" cy="12" r="3"/>',
    record: '<circle cx="12" cy="12" r="7.5"/><g class="m1"><circle cx="12" cy="12" r="3.6" fill="var(--rec)" stroke="none"/></g>',
    stop: '<circle cx="12" cy="12" r="7.5"/><rect x="9.4" y="9.4" width="5.2" height="5.2" rx="1" fill="var(--rec)" stroke="none"/>',
    folder: '<g class="m1"><path d="M3.5 7.5a2 2 0 0 1 2-2h4l2 2h7a2 2 0 0 1 2 2v7.5a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2z"/></g>',
    right: '<g class="m1"><path d="M9.5 6.5l5.5 5.5-5.5 5.5"/></g>',
    left: '<g class="m1"><path d="M14.5 6.5L9 12l5.5 5.5"/></g>',
    down: '<path d="M6.5 9.5l5.5 5.5 5.5-5.5"/>',
    close: '<g class="m1"><path d="M6.5 6.5l11 11M17.5 6.5l-11 11"/></g>',
    minimize: '<path d="M6 12h12"/>',
    maximize: '<rect x="6.5" y="6.5" width="11" height="11" rx="1.5"/>',
    trim: '<g class="m1"><path d="M9 4.5H6v15h3"/></g><g class="m2"><path d="M15 4.5h3v15h-3"/></g><path d="M12 8.5v7"/>',
    rename: '<g class="m1"><path d="M5 19l1-4 9.5-9.5a2.1 2.1 0 0 1 3 3L9 18z"/><path d="M13.5 7.5l3 3"/></g>',
    trash: '<g class="m1"><path d="M5 7h14M10 4.5h4"/></g><path d="M7 7l.8 11.2A1.9 1.9 0 0 0 9.7 20h4.6a1.9 1.9 0 0 0 1.9-1.8L17 7"/><path d="M10.5 11v5M13.5 11v5"/>',
    open: '<g class="m1"><path d="M13.5 5H19v5.5M19 5l-8 8"/></g><path d="M17 14v4a1.5 1.5 0 0 1-1.5 1.5h-9A1.5 1.5 0 0 1 5 18V8.5A1.5 1.5 0 0 1 6.5 7H10"/>',
    search: '<g class="m1"><circle cx="10.5" cy="10.5" r="5.5"/><path d="M15 15l4.5 4.5"/></g>',
    check: '<path class="draw" pathLength="1" d="M5.5 12.5l4 4 9-9"/>',
    alert: '<path d="M12 4.5l8.5 14.5h-17z"/><path d="M12 10v4"/><circle cx="12" cy="16.6" r=".9" fill="currentColor" stroke="none"/>',
    update: '<g class="m1"><path d="M12 4.5v10M8 10.5l4 4 4-4"/></g><path d="M5 15.5v2a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2v-2"/>',
  } as const;
  export type IconName = keyof typeof ICONS;
</script>

<script lang="ts">
  let { name, size = 20, stroke = 1.6 }: { name: IconName; size?: number; stroke?: number } = $props();
</script>

<svg
  class="ic ic-{name}"
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width={stroke}
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
>
  {@html ICONS[name]}
</svg>

<style>
  .ic {
    flex-shrink: 0;
    overflow: visible;
  }
  .ic :global(g),
  .ic :global(.draw) {
    transform-box: fill-box;
    transform-origin: center;
    transition: transform var(--dur) var(--ease);
  }

  /* Hover on the control that contains the icon brings it to life. */
  :global(:is(a, button, label):hover) > .ic-replay :global(.m1),
  :global(:is(a, button, label):hover) .ic-replay :global(.m1) {
    animation: ic-spin 0.7s var(--ease);
    transform-origin: 12px 12px;
    transform-box: view-box;
  }
  :global(:is(a, button, label):hover) .ic-film :global(.m1) {
    animation: ic-roll 0.6s var(--ease);
  }
  :global(:is(a, button, label):hover) .ic-sliders :global(.m1) {
    transform: translateX(-5px);
  }
  :global(:is(a, button, label):hover) .ic-sliders :global(.m2) {
    transform: translateX(5px);
  }
  /* The clapper arm lifts and snaps shut, pivoting on its left hinge. */
  .ic-clapper :global(.m1) {
    transform-box: view-box;
    transform-origin: 4.5px 10.3px;
  }
  :global(:is(a, button, label):hover) .ic-clapper :global(.m1) {
    animation: ic-clap 0.55s var(--ease);
  }
  :global(:is(a, button, label):hover) .ic-update :global(.m1) {
    animation: ic-drop 0.6s var(--ease);
  }
  :global(:is(a, button, label):hover) .ic-shot :global(.m1) {
    animation: ic-focus 0.6s var(--ease);
  }
  :global(:is(a, button, label):hover) .ic-record :global(.m1) {
    animation: ic-beat 0.9s var(--ease) infinite;
  }
  :global(:is(a, button, label):hover) .ic-folder :global(.m1) {
    transform: translateY(-1.5px) rotate(-4deg);
  }
  :global(:is(a, button, label):hover) .ic-right :global(.m1) {
    transform: translateX(2.5px);
  }
  :global(:is(a, button, label):hover) .ic-left :global(.m1) {
    transform: translateX(-2.5px);
  }
  :global(:is(a, button, label):hover) .ic-close :global(.m1) {
    transform: rotate(90deg);
  }
  :global(:is(a, button, label):hover) .ic-trim :global(.m1) {
    transform: translateX(-1.5px);
  }
  :global(:is(a, button, label):hover) .ic-trim :global(.m2) {
    transform: translateX(1.5px);
  }
  :global(:is(a, button, label):hover) .ic-rename :global(.m1) {
    transform: rotate(-8deg) translateY(-1px);
  }
  :global(:is(a, button, label):hover) .ic-trash :global(.m1) {
    transform: translateY(-1.5px) rotate(-10deg);
  }
  :global(:is(a, button, label):hover) .ic-open :global(.m1) {
    transform: translate(1.5px, -1.5px);
  }
  :global(:is(a, button, label):hover) .ic-search :global(.m1) {
    transform: rotate(-12deg) scale(1.06);
  }
  /* The check draws itself whenever it appears. */
  .ic-check :global(.draw) {
    stroke-dasharray: 1;
    animation: ic-draw 0.45s var(--ease) both;
  }

  @keyframes -global-ic-spin {
    from {
      transform: rotate(0deg);
    }
    to {
      transform: rotate(360deg);
    }
  }
  @keyframes -global-ic-roll {
    0% {
      transform: translateY(0);
    }
    45% {
      transform: translateY(2.5px);
      opacity: 0.3;
    }
    46% {
      transform: translateY(-2.5px);
    }
    100% {
      transform: translateY(0);
    }
  }
  @keyframes -global-ic-drop {
    0%,
    100% {
      transform: translateY(0);
    }
    45% {
      transform: translateY(3px);
    }
  }
  @keyframes -global-ic-clap {
    0%,
    100% {
      transform: rotate(0deg);
    }
    35% {
      transform: rotate(-16deg);
    }
    60% {
      transform: rotate(1.5deg);
    }
  }
  @keyframes -global-ic-focus {
    0%,
    100% {
      transform: scale(1);
    }
    45% {
      transform: scale(0.82);
    }
  }
  @keyframes -global-ic-beat {
    0%,
    100% {
      transform: scale(1);
    }
    40% {
      transform: scale(1.3);
    }
  }
  @keyframes -global-ic-draw {
    from {
      stroke-dashoffset: 1;
    }
    to {
      stroke-dashoffset: 0;
    }
  }
</style>
