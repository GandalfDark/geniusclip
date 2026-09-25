<script lang="ts">
  import { hotkeyParts } from '$lib/format';
  let { accel }: { accel: string } = $props();
</script>

<span class="keys">
  {#each hotkeyParts(accel) as k, i}
    {#if i > 0}<span class="plus">+</span>{/if}<kbd>{k}</kbd>
  {/each}
</span>

<style>
  .keys {
    display: inline-flex;
    align-items: center;
    gap: 3px;
  }
  .plus {
    color: var(--text-3);
    font-size: 11px;
  }
</style>
