<script lang="ts">
  import type { Snippet } from 'svelte';
  let { label, hint = '', children }: { label: string; hint?: string; children: Snippet } = $props();
</script>

<div class="row">
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
    min-height: 52px;
    padding: 10px 0;
    border-top: 1px solid var(--line);
  }
  .row:first-of-type {
    border-top: none;
  }
  .label {
    font-size: 13.5px;
    font-weight: 500;
  }
  .hint {
    color: var(--text-3);
    font-size: 12px;
    margin-top: 1px;
    max-width: 420px;
    overflow-wrap: anywhere;
  }
  .ctl {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-shrink: 0;
  }
</style>
