<script lang="ts">
  import Icon from './Icon.svelte';
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
    {#if conflict && !listening}<span class="warn"><Icon name="alert" size={15} /></span>{/if}
  </button>
  <button class="btn ghost sm icon" title={app.t('hk.clear')} disabled={!value} onclick={() => onchange('')}><Icon name="close" size={14} /></button>
</div>

<style>
  .hk {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .field {
    min-width: 190px;
    height: 32px;
    padding: 0 8px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    border-radius: var(--r);
    background: var(--bg);
    border: 1px solid var(--line-2);
  }
  .field:hover {
    border-color: #45454d;
  }
  .field.listening {
    border-color: var(--accent);
  }
  .field.conflict {
    border-color: color-mix(in srgb, var(--warn) 55%, transparent);
  }
  .listen {
    color: var(--accent);
    font-size: 12.5px;
  }
  .warn {
    display: flex;
    color: var(--warn);
  }
</style>
