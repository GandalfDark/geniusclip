// Renders the "Crystal" logo to src-tauri/app-icon.png (1024×1024).
// Then run: npx tauri icon src-tauri/app-icon.png
import { Resvg } from '@resvg/resvg-js';
import { writeFileSync } from 'node:fs';

const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64" width="1024" height="1024">
  <defs>
    <linearGradient id="g" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="#b3a4ff"/>
      <stop offset="0.55" stop-color="#9580ff"/>
      <stop offset="1" stop-color="#f472b6"/>
    </linearGradient>
  </defs>
  <g transform="translate(32 32) scale(1.12) translate(-32 -33)">
    <path d="M18 11h28l12 15-26 29L6 26z" fill="url(#g)"/>
    <path d="M6 26h52M18 11l7 15 7 29 7-29 7-15M25 26l7-15 7 15" fill="none" stroke="#fff" stroke-opacity="0.32" stroke-width="1.4" stroke-linejoin="round"/>
    <path d="M28 30v12l10-6z" fill="#fff"/>
  </g>
</svg>`;

const png = new Resvg(svg, { fitTo: { mode: 'width', value: 1024 } }).render().asPng();
writeFileSync(new URL('../src-tauri/app-icon.png', import.meta.url), png);
console.log('src-tauri/app-icon.png written');
