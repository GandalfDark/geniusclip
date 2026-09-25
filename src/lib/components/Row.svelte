<script lang="ts">
  import type { Snippet } from 'svelte';
  let { label, hint = '', children, stacked = false }: { label: string; hint?: string; children: Snippet; stacked?: boolean } = $props();
</script>

<div class="row" class:stacked>
  <div class="text">
    <div class="label">{label}</div>
    {#if hint}<div class="hint">{hint}</div>{/if}
  </div>
  <div class="ctl">{@render children()}</div>
</div>

<style>
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 24px;
    padding: 16px 0;
    border-top: 1px solid var(--line);
  }
  .row:first-of-type {
    border-top: none;
  }
  .row.stacked {
    flex-direction: column;
    align-items: stretch;
  }
  .label {
    font-weight: 600;
    font-size: 14px;
  }
  .hint {
    color: var(--text-2);
    font-size: 12.5px;
    margin-top: 2px;
    max-width: 440px;
  }
  .ctl {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-shrink: 0;
  }
</style>
