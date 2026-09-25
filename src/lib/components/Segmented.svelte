<script lang="ts" generics="T extends string | number">
  let { value, options, onchange, mono = false }: { value: T; options: { value: T; label: string }[]; onchange: (v: T) => void; mono?: boolean } =
    $props();
</script>

<div class="seg" class:mono role="radiogroup">
  {#each options as o}
    <button role="radio" aria-checked={o.value === value} class:active={o.value === value} onclick={() => onchange(o.value)}>
      {o.label}
    </button>
  {/each}
</div>

<style>
  .seg {
    display: inline-flex;
    border: 1px solid var(--line-2);
    border-radius: var(--r);
    overflow: hidden;
  }
  button {
    height: 30px;
    padding: 0 12px;
    font-size: 12.5px;
    font-weight: 500;
    color: var(--text-2);
    border-left: 1px solid var(--line-2);
    transition:
      background 0.1s,
      color 0.1s;
  }
  button:first-child {
    border-left: none;
  }
  .mono button {
    font-family: var(--mono);
    font-size: 12px;
  }
  button:hover {
    color: var(--text);
    background: var(--hover);
  }
  button.active {
    color: var(--accent-ink);
    background: var(--accent);
  }
</style>
