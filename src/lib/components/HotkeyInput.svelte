<script lang="ts" module>
  import { api } from '$lib/api';
  import { app } from '$lib/app.svelte';

  // Global hotkeys stay off while any field is listening, so pressing a
  // combo that is already bound records it instead of firing it.
  let listeners = 0;
  function suspend(on: boolean) {
    const was = listeners > 0;
    listeners += on ? 1 : -1;
    if (was === listeners > 0) return;
    // Conflicts aren't checked while suspended; take the fresh list on resume.
    api
      .setHotkeysSuspended(listeners > 0)
      .then((errs) => {
        if (listeners === 0 && errs) app.hotkeyErrors = errs;
      })
      .catch(() => {});
  }
</script>

<script lang="ts">
  import Icon from './Icon.svelte';
  import Keys from './Keys.svelte';
  import { rowIds } from './Row.svelte';

  let {
    value,
    onchange,
    conflict = false,
    label = '',
    taken,
  }: {
    value: string;
    onchange: (v: string) => void;
    conflict?: boolean;
    label?: string;
    /** The action another field already binds this combo to, if any. */
    taken?: (accel: string) => string | null;
  } = $props();
  let listening = $state(false);
  // Why the last key pressed wasn't taken (shown until the next one).
  let hint = $state('');
  $effect(() => {
    if (!listening) hint = '';
  });

  // Read as "<action> <combo>": the name (`label`, else the settings row's)
  // followed by the field's own content.
  const id = $props.id();
  const row = rowIds();
  let nameId = $derived(label ? `${id}-name` : row?.label);

  const MODS = ['Control', 'Alt', 'Shift', 'Meta'];
  const SOLO = /^(F\d{1,2}|Numpad(\d|Add|Subtract|Multiply|Divide|Decimal)|Insert|Home|End|PageUp|PageDown|Pause|ScrollLock|PrintScreen)$/;

  // Capture phase on window runs before any other key handler, so Esc here
  // cancels the rebinding without also closing the menu or the viewer.
  $effect(() => {
    if (!listening) return;
    window.addEventListener('keydown', onkeydown, true);
    suspend(true);
    return () => {
      window.removeEventListener('keydown', onkeydown, true);
      suspend(false);
    };
  });

  const RESERVED = ['Alt+F4', 'Alt+Tab', 'Alt+Shift+Tab', 'Control+Tab', 'Control+Shift+Tab', 'Alt+Escape', 'Control+Escape', 'Control+Alt+Delete'];

  function onkeydown(e: KeyboardEvent) {
    // Tab and Shift+Tab move on as usual (a keyboard user must be able to
    // leave the field) and are never bound.
    if (e.code === 'Tab' && !e.ctrlKey && !e.altKey && !e.metaKey) {
      listening = false;
      return;
    }
    e.preventDefault();
    e.stopImmediatePropagation();
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
    // Keys that don't type text work alone (numpad, F-keys, Insert, Home…);
    // letters, digits and the like need a modifier, or they'd fire while
    // typing in a game's chat.
    if (!mods.length && !SOLO.test(e.code)) {
      hint = app.t('hk.needMod');
      return;
    }
    const accel = [...mods, e.code].join('+');
    // Windows' own shortcuts: bound, they would stop working everywhere.
    if (RESERVED.includes(accel)) return;
    const other = taken?.(accel);
    if (other) {
      hint = app.t('hk.taken', { action: other });
      return;
    }
    onchange(accel);
    listening = false;
  }
</script>

<div class="hk">
  {#if label}<span id="{id}-name" hidden>{label}</span>{/if}
  <button
    class="field"
    class:listening
    class:conflict
    id="{id}-field"
    aria-labelledby={nameId ? `${nameId} ${id}-field` : undefined}
    aria-describedby={row?.hint}
    onclick={() => (listening = !listening)}
    onblur={() => (listening = false)}
  >
    {#if listening}
      <span class="listen" class:hint={!!hint}>{hint || app.t('hk.press')}</span>
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
  .listen.hint {
    color: var(--warn);
  }
</style>
