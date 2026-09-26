// Procedural GeniusClip logo: a "G" made of a replay arrow with a play mark
// in its counter. Shared by the UI (Logo.svelte, colors as CSS variables so it
// follows the accent) and scripts/make-icon.mjs (concrete colors for PNG/ICO).
//
// Coordinates are in a 100×100 box, angles in degrees with 0° = right and
// angles growing clockwise (screen coordinates).

const G = {
  cx: 48,
  cy: 55,
  r: 28, // ring centerline radius
  w: 21, // ring thickness
  start: -6, // ring end at the G bar (right, just above middle)
  end: 262, // ring end under the arrowhead (top)
  tilt: 14, // arrowhead turned upward from the tangent, for momentum
};

const rad = (d) => (d * Math.PI) / 180;
const f = (n) => Math.round(n * 100) / 100;
const pt = (a, r) => [G.cx + r * Math.cos(rad(a)), G.cy + r * Math.sin(rad(a))];

/** Annular sector from angle a0 clockwise to a1 between radii ri and ro. */
function sector(a0, a1, ri, ro) {
  const [x0, y0] = pt(a0, ro);
  const [x1, y1] = pt(a1, ro);
  const [x2, y2] = pt(a1, ri);
  const [x3, y3] = pt(a0, ri);
  const large = a1 - a0 > 180 ? 1 : 0;
  return `M${f(x0)} ${f(y0)}A${ro} ${ro} 0 ${large} 1 ${f(x1)} ${f(y1)}L${f(x2)} ${f(y2)}A${ri} ${ri} 0 ${large} 0 ${f(x3)} ${f(y3)}Z`;
}

function shapes() {
  const { cx, cy, r, w, start, end, tilt } = G;
  const ro = r + w / 2;
  const ri = r - w / 2;

  const ring = sector(start, end, ri, ro);

  // Arrowhead: its base lies exactly on the ring's radial end cut (so the
  // joint is seamless), flaring outward; the tip points clockwise and a bit
  // upward for momentum.
  const a = rad(end + 90 - tilt);
  const d = [Math.cos(a), Math.sin(a)];
  const [ix, iy] = pt(end - 1, ri - w * 0.24);
  const [ox, oy] = pt(end - 1, ro + w * 0.5);
  const [mx, my] = pt(end, r + w * 0.13);
  const len = w * 1.42;
  const arrow = `M${f(ix)} ${f(iy)}L${f(mx + d[0] * len)} ${f(my + d[1] * len)}L${f(ox)} ${f(oy)}Z`;

  // The G bar grows out of the ring's right end and points inward.
  const [, topY] = pt(start, ro);
  const barLeft = cx + ri * 0.55;
  const barH = w * 0.78;
  const barR = barH * 0.3;
  const barRight = cx + ro;
  const bar = `M${f(barRight)} ${f(topY)}H${f(barLeft + barR)}Q${f(barLeft)} ${f(topY)} ${f(barLeft)} ${f(topY + barR)}V${f(topY + barH - barR)}Q${f(barLeft)} ${f(topY + barH)} ${f(barLeft + barR)} ${f(topY + barH)}H${f(barRight - 4)}Z`;

  // Play mark in the counter, left of the bar.
  const tcx = cx - 3;
  const th = 17;
  const tw = 14.5;
  const tri = `M${f(tcx - tw / 2)} ${f(cy - th / 2)}L${f(tcx + tw / 2)} ${f(cy)}L${f(tcx - tw / 2)} ${f(cy + th / 2)}Z`;

  // Depth: a soft light rim on the upper left of the ring.
  const rim = sector(160, 250, ro - w * 0.22, ro);

  return { ring, arrow, bar, tri, rim };
}

/**
 * SVG markup (without the outer <svg>) for the logo.
 * `c` holds colors: any CSS color, including `var(--x)` in the browser.
 */
export function logoMarkup(c, id = 'gc') {
  const s = shapes();
  const stop = (o, col, op = 1) => `<stop offset="${o}" style="stop-color:${col};stop-opacity:${op}"/>`;
  return (
    `<defs>` +
    `<linearGradient id="${id}r" gradientUnits="userSpaceOnUse" x1="14" y1="10" x2="86" y2="92">${stop(0, c.light)}${stop(0.5, c.mid)}${stop(1, c.deep)}</linearGradient>` +
    `<linearGradient id="${id}t" gradientUnits="userSpaceOnUse" x1="36" y1="44" x2="56" y2="66">${stop(0, c.light)}${stop(1, c.mid)}</linearGradient>` +
    `</defs>` +
    `<path d="${s.ring}" fill="url(#${id}r)"/>` +
    `<path d="${s.bar}" fill="url(#${id}r)"/>` +
    `<path d="${s.arrow}" fill="url(#${id}r)"/>` +
    `<path d="${s.rim}" fill="#fff" fill-opacity="0.16"/>` +
    `<path d="${s.tri}" fill="url(#${id}t)" stroke="url(#${id}t)" stroke-width="4.5" stroke-linejoin="round"/>`
  );
}

// --- concrete colors (for PNG/ICO rendering) -------------------------------

const hex = (h) => [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16));
const toHex = (rgb) => '#' + rgb.map((v) => Math.round(v).toString(16).padStart(2, '0')).join('');
const mix = (a, b, t) => toHex(hex(a).map((v, i) => v + (hex(b)[i] - v) * t));

/** Same formulas as --logo-light / --logo-deep in app.css. */
export function logoColors(accent) {
  return { light: mix(accent, '#ffffff', 0.45), mid: accent, deep: mix(accent, '#140a33', 0.42) };
}
