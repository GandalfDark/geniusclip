<script lang="ts">
  let { checked = false, onchange, label = '', big = false }: { checked?: boolean; onchange: (v: boolean) => void; label?: string; big?: boolean } =
    $props();
</script>

<button class="sw" class:on={checked} class:big role="switch" aria-checked={checked} aria-label={label} onclick={() => onchange(!checked)}>
  <span class="knob"></span>
</button>

<style>
  .sw {
    --w: 44px;
    --h: 26px;
    position: relative;
    width: var(--w);
    height: var(--h);
    border-radius: 99px;
    background: rgba(167, 139, 250, 0.16);
    border: 1px solid var(--line-2);
    transition: background 0.2s;
    flex-shrink: 0;
  }
  .sw.big {
    --w: 58px;
    --h: 32px;
  }
  .sw.on {
    background: var(--accent-grad);
    border-color: transparent;
  }
  .knob {
    position: absolute;
    top: 3px;
    left: 3px;
    width: calc(var(--h) - 8px);
    height: calc(var(--h) - 8px);
    border-radius: 50%;
    background: #f5f0ff;
    box-shadow: 0 2px 6px rgba(0, 0, 0, 0.35);
    transition: transform 0.25s var(--ease);
  }
  .sw.on .knob {
    transform: translateX(calc(var(--w) - var(--h)));
  }
</style>
