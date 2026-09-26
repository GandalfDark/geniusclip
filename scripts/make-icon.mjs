// Renders the procedural logo (src/lib/logo.js) to PNGs:
//  * src-tauri/app-icon.png            1024 px, default accent → `npx tauri icon`
//  * src-tauri/icons/accent/<id>.png   256 px per accent → tray/window icon at runtime
// Usage: node scripts/make-icon.mjs && npx tauri icon src-tauri/app-icon.png
import { Resvg } from '@resvg/resvg-js';
import { mkdirSync, writeFileSync } from 'node:fs';
import { logoMarkup, logoColors } from '../src/lib/logo.js';
import { ACCENTS } from '../src/lib/accents.ts';

const render = (accent, size) => {
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100" width="${size}" height="${size}">${logoMarkup(logoColors(accent))}</svg>`;
  return new Resvg(svg, { fitTo: { mode: 'width', value: size } }).render().asPng();
};

const root = new URL('../src-tauri/', import.meta.url);
writeFileSync(new URL('app-icon.png', root), render(ACCENTS.violet.color, 1024));
mkdirSync(new URL('icons/accent/', root), { recursive: true });
for (const [id, acc] of Object.entries(ACCENTS)) {
  writeFileSync(new URL(`icons/accent/${id}.png`, root), render(acc.color, 256));
}
console.log('icons rendered:', Object.keys(ACCENTS).join(', '));
