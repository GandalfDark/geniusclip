<script lang="ts">
  import { logoMarkup } from '$lib/logo.js';

  let { size = 30 }: { size?: number } = $props();
  const id = `lg${Math.random().toString(36).slice(2, 8)}`;
  // Colors come from the accent, so the logo recolors with the theme.
  const markup = logoMarkup({ light: 'var(--logo-light)', mid: 'var(--accent)', deep: 'var(--logo-deep)' }, id);
</script>

<svg width={size} height={size} viewBox="0 0 100 100" aria-label="GeniusClip">{@html markup}</svg>
