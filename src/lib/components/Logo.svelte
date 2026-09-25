<script lang="ts">
  let { size = 36 }: { size?: number } = $props();
  const id = `lg${Math.random().toString(36).slice(2, 8)}`;
</script>

<svg width={size} height={size} viewBox="0 0 48 48" aria-label="GeniusClip">
  <defs>
    <linearGradient id={id} x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="var(--accent-a)" />
      <stop offset="1" stop-color="var(--accent-b)" />
    </linearGradient>
  </defs>
  <circle
    cx="24"
    cy="24"
    r="17"
    fill="none"
    stroke="url(#{id})"
    stroke-width="5"
    stroke-linecap="round"
    stroke-dasharray="84 23"
    transform="rotate(-58 24 24)"
  />
  <path d="M20.5 17.5 L31 24 L20.5 30.5 Z" fill="url(#{id})" stroke="url(#{id})" stroke-width="3" stroke-linejoin="round" />
</svg>
