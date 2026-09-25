<script lang="ts">
  import { fly } from 'svelte/transition';
  import { app } from '$lib/app.svelte';
</script>

<div class="notices">
  {#each app.notices as n (n.id)}
    <div class="notice {n.tone}" transition:fly={{ y: 8, duration: 160 }}>{n.text}</div>
  {/each}
</div>

<style>
  .notices {
    position: fixed;
    right: 20px;
    bottom: 20px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    z-index: 100;
    max-width: 420px;
  }
  .notice {
    padding: 10px 14px 10px 12px;
    border-radius: var(--r);
    background: var(--panel-2);
    border: 1px solid var(--line-2);
    border-left: 3px solid var(--text-3);
    font-size: 13px;
    user-select: text;
  }
  .notice.error {
    border-left-color: var(--danger);
  }
  .notice.ok {
    border-left-color: var(--accent);
  }
</style>
