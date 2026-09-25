<script lang="ts">
  import { hotkeyParts } from '$lib/format';
  /**
   * caps  — separate key caps (settings)
   * chip  — one small chip "Alt F8" inside buttons
   * plain — dimmed text
   */
  let { accel, variant = 'caps' }: { accel: string; variant?: 'caps' | 'chip' | 'plain' } = $props();
</script>

{#if variant === 'caps'}
  <span class="keys">
    {#each hotkeyParts(accel) as k}<kbd>{k}</kbd>{/each}
  </span>
{:else}
  <span class="{variant} mono">{hotkeyParts(accel).join(' ')}</span>
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
  .chip {
    font-size: 10.5px;
    font-weight: 500;
    padding: 1px 6px;
    border-radius: 4px;
    border: 1px solid #3a3a42;
    color: var(--text-2);
  }
  :global(.btn.primary) .chip {
    border-color: transparent;
    background: rgba(21, 12, 38, 0.2);
    color: var(--accent-ink);
  }
</style>
