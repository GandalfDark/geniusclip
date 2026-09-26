<script lang="ts">
  let { checked = false, onchange, label = '' }: { checked?: boolean; onchange: (v: boolean) => void; label?: string } = $props();
</script>

<button class="sw" class:on={checked} role="switch" aria-checked={checked} aria-label={label} onclick={() => onchange(!checked)}>
  <span class="knob"></span>
</button>

<style>
  .sw {
    position: relative;
    width: 36px;
    height: 20px;
    border-radius: 10px;
    background: #2e2e35;
    transition: background var(--dur) var(--ease);
    flex-shrink: 0;
  }
  .sw.on {
    background: linear-gradient(90deg, var(--accent), var(--accent-2));
  }
  .knob {
    position: absolute;
    top: 3px;
    left: 3px;
    width: 14px;
    height: 14px;
    border-radius: 7px;
    background: #d9d9de;
    transition:
      transform var(--dur) var(--ease),
      width var(--dur-fast) var(--ease),
      background var(--dur) var(--ease);
  }
  .sw.on .knob {
    transform: translateX(16px);
    background: var(--bg);
  }
  /* The knob stretches while pressed, like a real toggle. */
  .sw:active .knob {
    width: 18px;
  }
  .sw.on:active .knob {
    transform: translateX(12px);
  }
</style>
