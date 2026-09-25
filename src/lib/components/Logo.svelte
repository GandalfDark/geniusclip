<script lang="ts">
  let { size = 28 }: { size?: number } = $props();
  const id = `gem${Math.random().toString(36).slice(2, 8)}`;
</script>

<!-- "Crystal": a brilliant-cut gem (the "genius" clip) with a play mark.
     Brand colors are fixed and do not follow the accent setting. -->
<svg width={size} height={size} viewBox="0 0 64 64" aria-label="GeniusClip">
  <defs>
    <linearGradient id={id} x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="#b3a4ff" />
      <stop offset="0.55" stop-color="#9580ff" />
      <stop offset="1" stop-color="#f472b6" />
    </linearGradient>
  </defs>
  <path d="M18 11h28l12 15-26 29L6 26z" fill="url(#{id})" />
  <path
    d="M6 26h52M18 11l7 15 7 29 7-29 7-15M25 26l7-15 7 15"
    fill="none"
    stroke="#fff"
    stroke-opacity="0.32"
    stroke-width="1.4"
    stroke-linejoin="round"
  />
  <path d="M28 30v12l10-6z" fill="#fff" />
</svg>
