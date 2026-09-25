<script lang="ts" generics="T extends string | number">
  let { value, options, onchange }: { value: T; options: { value: T; label: string }[]; onchange: (v: T) => void } = $props();
</script>

<div class="seg" role="radiogroup">
  {#each options as o}
    <button role="radio" aria-checked={o.value === value} class:active={o.value === value} onclick={() => onchange(o.value)}>
      {o.label}
    </button>
  {/each}
</div>

<style>
  .seg {
    display: inline-flex;
    padding: 3px;
    gap: 2px;
    border-radius: 11px;
    background: rgba(13, 10, 23, 0.6);
    border: 1px solid var(--line);
  }
  button {
    height: 30px;
    padding: 0 14px;
    border-radius: 8px;
    font-size: 13px;
    font-weight: 600;
    color: var(--text-2);
    transition:
      background 0.15s,
      color 0.15s;
  }
  button:hover {
    color: var(--text);
  }
  button.active {
    background: var(--active);
    color: var(--text);
    box-shadow: inset 0 0 0 1px var(--line-2);
  }
</style>
