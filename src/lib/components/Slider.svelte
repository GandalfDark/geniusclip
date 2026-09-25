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
  <span class="val mono">{format(value)}</span>
</div>

<style>
  .slider {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  input {
    flex: 1;
    appearance: none;
    height: 4px;
    border-radius: 2px;
    background: linear-gradient(var(--accent), var(--accent)) 0 / var(--pct) 100% no-repeat, #2e2e35;
    cursor: pointer;
  }
  input::-webkit-slider-thumb {
    appearance: none;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: #ececef;
    border: 3px solid var(--bg);
    box-shadow: 0 0 0 1px var(--line-2);
  }
  .val {
    min-width: 54px;
    text-align: right;
    font-size: 12.5px;
    color: var(--text-2);
  }
</style>
