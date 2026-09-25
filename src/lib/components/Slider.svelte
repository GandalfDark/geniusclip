<script lang="ts">
  let {
    value,
    min,
    max,
    step = 1,
    onchange,
    format = (v: number) => String(v),
    width = '260px',
  }: { value: number; min: number; max: number; step?: number; onchange: (v: number) => void; format?: (v: number) => string; width?: string } =
    $props();
  let pct = $derived(((value - min) / (max - min)) * 100);
</script>

<div class="slider" style:width>
  <input type="range" {min} {max} {step} {value} style:--pct="{pct}%" oninput={(e) => onchange(Number((e.currentTarget as HTMLInputElement).value))} />
  <span class="val">{format(value)}</span>
</div>

<style>
  .slider {
    display: flex;
    align-items: center;
    gap: 14px;
  }
  input {
    flex: 1;
    appearance: none;
    height: 6px;
    border-radius: 6px;
    background: linear-gradient(90deg, var(--accent-a), var(--accent-b)) 0 / var(--pct) 100% no-repeat, rgba(167, 139, 250, 0.16);
    cursor: pointer;
  }
  input::-webkit-slider-thumb {
    appearance: none;
    width: 18px;
    height: 18px;
    border-radius: 50%;
    background: #f5f0ff;
    box-shadow:
      0 0 0 4px color-mix(in srgb, var(--accent-a) 30%, transparent),
      0 2px 8px rgba(0, 0, 0, 0.4);
    transition: transform 0.1s;
  }
  input:active::-webkit-slider-thumb {
    transform: scale(1.12);
  }
  .val {
    min-width: 58px;
    text-align: right;
    font-family: var(--font-display);
    font-size: 13px;
    font-weight: 500;
  }
</style>
