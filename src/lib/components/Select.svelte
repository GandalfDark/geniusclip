<script lang="ts">
  import IconChevronDown from '@tabler/icons-svelte-runes/icons/chevron-down';
  let {
    value,
    options,
    onchange,
    width = '260px',
  }: { value: string; options: { value: string; label: string }[]; onchange: (v: string) => void; width?: string } = $props();
</script>

<label class="sel" style:width>
  <select {value} onchange={(e) => onchange((e.currentTarget as HTMLSelectElement).value)}>
    {#each options as o}
      <option value={o.value}>{o.label}</option>
    {/each}
  </select>
  <IconChevronDown size={16} class="chev" />
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
    height: 38px;
    padding: 0 36px 0 12px;
    border-radius: var(--r);
    background: rgba(13, 10, 23, 0.6);
    border: 1px solid var(--line-2);
    font-size: 13.5px;
    font-weight: 600;
    cursor: pointer;
    text-overflow: ellipsis;
  }
  select:hover {
    border-color: rgba(196, 181, 253, 0.3);
  }
  option {
    background: #1a1330;
  }
  .sel :global(.chev) {
    position: absolute;
    right: 12px;
    pointer-events: none;
    color: var(--text-2);
  }
</style>
