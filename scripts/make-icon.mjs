// Prepares the logo assets from the source PNGs in assets/logo/<accent>.png
// (512 px, transparent):
//  * src/lib/assets/logo/<id>.png     128 px, shown in the UI
//  * src-tauri/icons/accent/<id>.png  256 px, tray/window icon at runtime
//  * src-tauri/app-icon.png           512 px violet → `npx tauri icon`
// Usage: node scripts/make-icon.mjs && npx tauri icon src-tauri/app-icon.png
import { Resvg } from '@resvg/resvg-js';
import { copyFileSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';

const ids = ['violet', 'red', 'lime', 'cyan', 'amber', 'mono'];
const root = new URL('../', import.meta.url);

// resvg does high-quality raster scaling when an image is drawn into a smaller SVG.
const scale = (png, size) => {
  const data = `data:image/png;base64,${png.toString('base64')}`;
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${size}" height="${size}"><image href="${data}" width="${size}" height="${size}"/></svg>`;
  return new Resvg(svg, { imageRendering: 0 }).render().asPng();
};

mkdirSync(new URL('src/lib/assets/logo/', root), { recursive: true });
mkdirSync(new URL('src-tauri/icons/accent/', root), { recursive: true });
for (const id of ids) {
  const src = readFileSync(new URL(`assets/logo/${id}.png`, root));
  writeFileSync(new URL(`src/lib/assets/logo/${id}.png`, root), scale(src, 128));
  writeFileSync(new URL(`src-tauri/icons/accent/${id}.png`, root), scale(src, 256));
}
copyFileSync(new URL('assets/logo/violet.png', root), new URL('src-tauri/app-icon.png', root));
console.log('logo assets prepared:', ids.join(', '));
