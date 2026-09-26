<script lang="ts">
  import violet from '$lib/assets/logo/violet.png';
  import red from '$lib/assets/logo/red.png';
  import lime from '$lib/assets/logo/lime.png';
  import cyan from '$lib/assets/logo/cyan.png';
  import amber from '$lib/assets/logo/amber.png';
  import mono from '$lib/assets/logo/mono.png';
  import { app } from '$lib/app.svelte';

  // One logo image per accent preset, so the logo follows the theme color.
  const LOGOS: Record<string, string> = { violet, red, lime, cyan, amber, mono };

  let { size = 34 }: { size?: number } = $props();
  let src = $derived(LOGOS[app.settings?.accent ?? 'violet'] ?? violet);
</script>

<img {src} width={size} height={size} alt="GeniusClip" draggable="false" />

<style>
  img {
    display: block;
    object-fit: contain;
  }
</style>
