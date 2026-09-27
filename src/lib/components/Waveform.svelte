<script lang="ts">
  // One audio lane of the trim editor: the track's waveform at its current
  // volume, bright inside the selection, plus a dashed volume line.
  let {
    peaks,
    gain,
    muted,
    from,
    to,
  }: { peaks: number[]; gain: number; muted: boolean; from: number; to: number } = $props();

  let canvas: HTMLCanvasElement;
  let w = $state(0);
  let h = $state(0);
  // Redrawn on every drag step: theme colours are read once (the accent
  // can't change while the viewer is open) and the backing store is only
  // resized when the lane is.
  let colors: { accent: string; warn: string } | null = null;

  // Levels are drawn on a dB scale so quiet voices stay visible and
  // a 2× boost reads as a clear step up.
  const RANGE_DB = 48;
  const BAR = 2;
  const GAP = 1;

  const height = (level: number) => (level <= 0 ? 0 : Math.max(0, Math.min(1, (20 * Math.log10(level) + RANGE_DB) / RANGE_DB)));

  $effect(() => {
    if (!canvas || w === 0 || h === 0) return;
    const dpr = window.devicePixelRatio || 1;
    const cw = Math.round(w * dpr);
    const ch = Math.round(h * dpr);
    if (canvas.width !== cw || canvas.height !== ch) {
      canvas.width = cw;
      canvas.height = ch;
    }
    const g = canvas.getContext('2d');
    if (!g) return;
    g.setTransform(dpr, 0, 0, dpr, 0, 0);
    g.clearRect(0, 0, w, h);

    if (!colors) {
      const css = getComputedStyle(canvas);
      colors = { accent: css.getPropertyValue('--accent').trim() || '#9580ff', warn: css.getPropertyValue('--warn').trim() || '#f5b83d' };
    }
    const { accent, warn } = colors;
    const mid = h / 2;
    const n = peaks.length;
    for (let x = 0; x < w; x += BAR + GAP) {
      let pk = 0;
      if (n > 0) {
        const a = Math.floor((x / w) * n);
        const b = Math.max(a + 1, Math.floor(((x + BAR + GAP) / w) * n));
        for (let i = a; i < b && i < n; i++) pk = Math.max(pk, peaks[i]);
      }
      const level = pk * gain;
      const bh = Math.max(0.75, height(level) * (mid - 2));
      const inside = x + BAR > from * w && x < to * w;
      g.fillStyle = muted ? '#2a2a31' : !inside ? '#3a3a43' : level >= 0.99 ? warn : accent;
      g.globalAlpha = muted || inside ? 1 : 0.9;
      g.fillRect(x, mid - bh, BAR, bh * 2);
    }
    g.globalAlpha = 1;

    // Volume line: 0% at the centre, 200% at the top edge.
    if (!muted) {
      const y = Math.round(mid - (gain / 2) * (mid - 2)) + 0.5;
      g.strokeStyle = 'rgba(255, 255, 255, 0.4)';
      g.lineWidth = 1;
      g.setLineDash([3, 3]);
      g.beginPath();
      g.moveTo(0, y);
      g.lineTo(w, y);
      g.stroke();
    }
  });
</script>

<div class="wave" bind:clientWidth={w} bind:clientHeight={h}>
  <canvas bind:this={canvas}></canvas>
</div>

<style>
  .wave {
    position: absolute;
    inset: 0;
  }
  canvas {
    display: block;
    width: 100%;
    height: 100%;
  }
</style>
