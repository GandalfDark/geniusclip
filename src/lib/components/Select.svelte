<script lang="ts">
  import Icon from './Icon.svelte';
  import { rowIds } from './Row.svelte';
  // Named by `label`, else by the settings row it sits in.
  let {
    value,
    options,
    onchange,
    width = '260px',
    label = '',
  }: { value: string; options: { value: string; label: string }[]; onchange: (v: string) => void; width?: string; label?: string } = $props();
  const row = rowIds();
</script>

<label class="sel" style:width>
  <select
    {value}
    aria-label={label || undefined}
    aria-labelledby={label ? undefined : row?.label}
    aria-describedby={row?.hint}
    onchange={(e) => onchange((e.currentTarget as HTMLSelectElement).value)}
  >
    {#each options as o}
      <option value={o.value}>{o.label}</option>
    {/each}
  </select>
  <span class="chev"><Icon name="down" size={15} /></span>
</label>

<style>
  .sel {
    position: relative;
    display: inline-flex;
    align-items: center;
  }
  select {
    appearance: none;
    width: 100%;
    height: 32px;
    padding: 0 32px 0 10px;
    border-radius: var(--r);
    background: var(--bg);
    border: 1px solid var(--line-2);
    font-size: 13px;
    cursor: pointer;
    text-overflow: ellipsis;
  }
  select:hover {
    border-color: #45454d;
  }
  option {
    background: var(--panel-2);
  }
  .chev {
    position: absolute;
    right: 9px;
    display: flex;
    pointer-events: none;
    color: var(--text-3);
  }
</style>
