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
    transition: background 0.15s;
    flex-shrink: 0;
  }
  .sw.on {
    background: var(--accent);
  }
  .knob {
    position: absolute;
    top: 3px;
    left: 3px;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: #d9d9de;
    transition: transform 0.18s cubic-bezier(0.3, 0.7, 0.4, 1);
  }
  .sw.on .knob {
    transform: translateX(16px);
    background: var(--bg);
  }
</style>
