<script lang="ts">
  import '../app.css';
  import { app } from '$lib/app.svelte';
  let { children } = $props();

  // Every window (main, in-game menu): language-specific glyph shapes (Han
  // characters in ja vs zh), fonts (see app.css) and screen readers follow
  // the <html lang>.
  $effect(() => {
    if (app.snapshot) document.documentElement.lang = app.lang;
  });
</script>

{@render children()}
