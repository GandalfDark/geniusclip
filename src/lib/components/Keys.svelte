<script lang="ts">
  import { hotkeyParts } from '$lib/format';
  /** `plain` renders "Alt F8" as text (inside buttons); otherwise as key caps. */
  let { accel, plain = false }: { accel: string; plain?: boolean } = $props();
</script>

{#if plain}
  <span class="plain mono">{hotkeyParts(accel).join(' ')}</span>
{:else}
  <span class="keys">
    {#each hotkeyParts(accel) as k}<kbd>{k}</kbd>{/each}
  </span>
{/if}

<style>
  .keys {
    display: inline-flex;
    gap: 3px;
  }
  .plain {
    font-size: 11px;
    font-weight: 500;
    opacity: 0.65;
  }
</style>
