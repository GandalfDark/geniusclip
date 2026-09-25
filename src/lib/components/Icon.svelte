<script lang="ts" module>
  // GeniusClip's own icon set: 24px grid, round caps, one stroke weight.
  // Filled parts use `fill="currentColor"`; the record dot uses --rec.
  const ICONS = {
    replay: '<path d="M4.5 12a7.5 7.5 0 1 0 2.2-5.3"/><path d="M4.5 4.5v3.2h3.2"/><circle cx="12" cy="12" r="1.7" fill="currentColor" stroke="none"/>',
    film: '<rect x="4" y="5" width="16" height="14" rx="2"/><path d="M8.5 5v14M15.5 5v14M4 9.5h4.5M4 14.5h4.5M15.5 9.5H20M15.5 14.5H20"/>',
    sliders: '<path d="M4 7.5h9M18 7.5h2M4 16.5h2M11 16.5h9"/><circle cx="15.5" cy="7.5" r="2.3"/><circle cx="8.5" cy="16.5" r="2.3"/>',
    save: '<path d="M12 4v10M8 10l4 4 4-4M5 19.5h14"/>',
    shot: '<path d="M4 8.5V5h3.5M16.5 5H20v3.5M20 15.5V19h-3.5M7.5 19H4v-3.5"/><circle cx="12" cy="12" r="3"/>',
    record: '<circle cx="12" cy="12" r="7.5"/><circle cx="12" cy="12" r="3.6" fill="var(--rec)" stroke="none"/>',
    stop: '<circle cx="12" cy="12" r="7.5"/><rect x="9.4" y="9.4" width="5.2" height="5.2" rx="1" fill="var(--rec)" stroke="none"/>',
    folder: '<path d="M3.5 7.5a2 2 0 0 1 2-2h4l2 2h7a2 2 0 0 1 2 2v7.5a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2z"/>',
    right: '<path d="M9.5 6.5l5.5 5.5-5.5 5.5"/>',
    left: '<path d="M14.5 6.5L9 12l5.5 5.5"/>',
    down: '<path d="M6.5 9.5l5.5 5.5 5.5-5.5"/>',
    close: '<path d="M6.5 6.5l11 11M17.5 6.5l-11 11"/>',
    minimize: '<path d="M6 12h12"/>',
    maximize: '<rect x="6.5" y="6.5" width="11" height="11" rx="1.5"/>',
    trim: '<path d="M9 4.5H6v15h3M15 4.5h3v15h-3"/><path d="M12 8.5v7"/>',
    rename: '<path d="M5 19l1-4 9.5-9.5a2.1 2.1 0 0 1 3 3L9 18z"/><path d="M13.5 7.5l3 3"/>',
    trash: '<path d="M5 7h14M10 4.5h4M7 7l.8 11.2A1.9 1.9 0 0 0 9.7 20h4.6a1.9 1.9 0 0 0 1.9-1.8L17 7"/><path d="M10.5 11v5M13.5 11v5"/>',
    open: '<path d="M13.5 5H19v5.5M19 5l-8 8"/><path d="M17 14v4a1.5 1.5 0 0 1-1.5 1.5h-9A1.5 1.5 0 0 1 5 18V8.5A1.5 1.5 0 0 1 6.5 7H10"/>',
    search: '<circle cx="10.5" cy="10.5" r="5.5"/><path d="M15 15l4.5 4.5"/>',
    check: '<path d="M5.5 12.5l4 4 9-9"/>',
    alert: '<path d="M12 4.5l8.5 14.5h-17z"/><path d="M12 10v4"/><circle cx="12" cy="16.6" r=".9" fill="currentColor" stroke="none"/>',
    update: '<path d="M12 4.5v10M8 10.5l4 4 4-4"/><path d="M5 15.5v2a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2v-2"/>',
  } as const;
  export type IconName = keyof typeof ICONS;
</script>

<script lang="ts">
  let { name, size = 20, stroke = 1.6 }: { name: IconName; size?: number; stroke?: number } = $props();
</script>

<svg
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
