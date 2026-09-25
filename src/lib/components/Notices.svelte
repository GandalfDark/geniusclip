<script lang="ts">
  import { fly } from 'svelte/transition';
  import { app } from '$lib/app.svelte';
</script>

<div class="notices">
  {#each app.notices as n (n.id)}
    <div class="notice {n.tone}" transition:fly={{ y: 12, duration: 220 }}>{n.text}</div>
  {/each}
</div>

<style>
  .notices {
    position: fixed;
    right: 24px;
    bottom: 24px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    z-index: 100;
    max-width: 420px;
  }
  .notice {
    padding: 12px 16px;
    border-radius: var(--r);
    background: var(--panel-solid);
    border: 1px solid var(--line-2);
    box-shadow: 0 18px 40px -16px rgba(0, 0, 0, 0.7);
    font-weight: 600;
    font-size: 13px;
    user-select: text;
  }
  .notice.error {
    border-color: color-mix(in srgb, var(--danger) 45%, transparent);
  }
  .notice.ok {
    border-color: color-mix(in srgb, var(--ok) 45%, transparent);
  }
</style>
