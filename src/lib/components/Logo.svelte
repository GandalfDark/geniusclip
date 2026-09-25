<script lang="ts">
  let { size = 24 }: { size?: number } = $props();
</script>

<!-- Replay loop with a record dot: the buffer that keeps rolling. -->
<svg width={size} height={size} viewBox="0 0 24 24" fill="none" aria-label="GeniusClip">
  <path d="M4.5 12a7.5 7.5 0 1 0 2.2-5.3" stroke="var(--accent)" stroke-width="2" stroke-linecap="round" />
  <path d="M4.5 4.2v3.5H8" stroke="var(--accent)" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" />
  <circle cx="12" cy="12" r="2.2" fill="var(--rec)" />
</svg>
