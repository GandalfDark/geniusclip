<script lang="ts">
  import IconX from '@tabler/icons-svelte-runes/icons/x';
  import IconAlertTriangle from '@tabler/icons-svelte-runes/icons/alert-triangle';
  import Keys from './Keys.svelte';
  import { app } from '$lib/app.svelte';

  let { value, onchange, conflict = false }: { value: string; onchange: (v: string) => void; conflict?: boolean } = $props();
  let listening = $state(false);

  const MODS = ['Control', 'Alt', 'Shift', 'Meta'];

  function onkeydown(e: KeyboardEvent) {
    if (!listening) return;
    e.preventDefault();
    e.stopPropagation();
    if (e.code === 'Escape') {
      listening = false;
      return;
    }
    if (e.code === 'Backspace' || e.code === 'Delete') {
      onchange('');
      listening = false;
      return;
    }
    if (MODS.includes(e.key)) return;
    const mods = [];
    if (e.ctrlKey) mods.push('Control');
    if (e.altKey) mods.push('Alt');
    if (e.shiftKey) mods.push('Shift');
    if (e.metaKey) mods.push('Super');
    const isF = /^F\d{1,2}$/.test(e.code);
    if (!mods.length && !isF) return; // plain letters would fire while typing in games/chat
    onchange([...mods, e.code].join('+'));
    listening = false;
  }
</script>

<svelte:window {onkeydown} />

<div class="hk">
  <button class="field" class:listening class:conflict onclick={() => (listening = !listening)} onblur={() => (listening = false)}>
    {#if listening}
      <span class="listen">{app.t('hk.press')}</span>
    {:else if value}
      <Keys accel={value} />
    {:else}
      <span class="faint">{app.t('hk.none')}</span>
    {/if}
    {#if conflict && !listening}
      <IconAlertTriangle size={16} class="warn" />
    {/if}
  </button>
  {#if value}
    <button class="btn ghost sm icon" title={app.t('hk.clear')} onclick={() => onchange('')}><IconX size={15} /></button>
  {/if}
</div>

<style>
  .hk {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .field {
    min-width: 200px;
    height: 38px;
    padding: 0 12px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    border-radius: var(--r);
    background: rgba(13, 10, 23, 0.6);
    border: 1px solid var(--line-2);
    transition: border-color 0.15s;
  }
  .field:hover {
    border-color: rgba(196, 181, 253, 0.3);
  }
  .field.listening {
    border-color: var(--accent-a);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent-a) 22%, transparent);
  }
  .field.conflict {
    border-color: color-mix(in srgb, var(--warn) 60%, transparent);
  }
  .listen {
    color: var(--accent-a);
    font-weight: 600;
    font-size: 13px;
    animation: blink 1.2s infinite;
  }
  .field :global(.warn) {
    color: var(--warn);
  }
  @keyframes blink {
    50% {
      opacity: 0.45;
    }
  }
</style>
