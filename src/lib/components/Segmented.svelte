<script lang="ts" generics="T extends string | number">
  let { value, options, onchange, mono = false }: { value: T; options: { value: T; label: string }[]; onchange: (v: T) => void; mono?: boolean } =
    $props();

  let root: HTMLDivElement;
  let btns: HTMLButtonElement[] = $state([]);
  let pill = $state({ x: 0, w: 0 });
  // Only animate after the first placement, so the pill doesn't slide in on load.
  let animated = $state(false);

  function place() {
    const b = btns[options.findIndex((o) => o.value === value)];
    if (b) pill = { x: b.offsetLeft, w: b.offsetWidth };
  }

  $effect(() => {
    void value;
    void options.length;
    place();
    if (!animated) requestAnimationFrame(() => (animated = true));
  });

  $effect(() => {
    // Labels change width when fonts load or the language switches.
    const ro = new ResizeObserver(place);
    ro.observe(root);
    document.fonts?.ready.then(place);
    return () => ro.disconnect();
  });
</script>

<div class="seg" class:mono bind:this={root} role="radiogroup">
  <span class="pill" class:animated style:transform="translateX({pill.x}px)" style:width="{pill.w}px"></span>
  {#each options as o, i}
    <button bind:this={btns[i]} role="radio" aria-checked={o.value === value} class:active={o.value === value} onclick={() => onchange(o.value)}>
      {o.label}
    </button>
  {/each}
</div>

<style>
  .seg {
    position: relative;
    display: inline-flex;
    padding: 2px;
    border: 1px solid var(--line-2);
    border-radius: var(--r);
  }
  .pill {
    position: absolute;
    top: 2px;
    bottom: 2px;
    left: 0;
    border-radius: 6px;
    background: linear-gradient(135deg, var(--accent), var(--accent-2));
  }
  .pill.animated {
    transition:
      transform var(--dur) var(--ease),
      width var(--dur) var(--ease);
  }
  button {
    position: relative;
    height: 28px;
    padding: 0 12px;
    font-size: 12.5px;
    font-weight: 500;
    color: var(--text-2);
    border-radius: 6px;
    transition: color var(--dur) var(--ease);
  }
  .mono button {
    font-family: var(--mono);
    font-size: 12px;
  }
  button:hover:not(.active) {
    color: var(--text);
  }
  button.active {
    color: var(--accent-ink);
  }
</style>
