// Renders the website's app screenshots and link-preview image with headless
// Edge. The app screens come from the UI's browser preview with its mock
// backend, so no real clips of anyone end up on the site.
//
//   npm run dev                                   (UI preview on :1420)
//   python -m http.server 8090 --directory site   (the site, for og.html)
//   node scripts/site-shots.mjs
import puppeteer from 'puppeteer-core';
import { execFileSync } from 'node:child_process';
import { mkdirSync } from 'node:fs';

const EDGE = 'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe';
const APP = 'http://localhost:1420';
const SITE = 'http://127.0.0.1:8090';
const TMP = 'target/shots';
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
mkdirSync(TMP, { recursive: true });

const browser = await puppeteer.launch({
  executablePath: EDGE,
  headless: true,
  args: ['--autoplay-policy=no-user-gesture-required', '--hide-scrollbars', '--mute-audio'],
});
const page = await browser.newPage();
await page.setViewport({ width: 1200, height: 780, deviceScaleFactor: 1.5 });

for (const lang of ['ru', 'en']) {
  await page.goto(`${APP}/?lang=${lang}&clean`, { waitUntil: 'networkidle0' });
  await sleep(1800);
  await page.screenshot({ path: `${TMP}/${lang}-home.png` });

  await page.goto(`${APP}/gallery?lang=${lang}&clean`, { waitUntil: 'networkidle0' });
  await sleep(1800);
  await page.screenshot({ path: `${TMP}/${lang}-gallery.png` });

  // Trim: open the video clip, select 0:04–0:17, raise the mic a bit.
  await page.click('[data-path="/dev-clip.mp4"] .thumb');
  await sleep(1400);
  await page.click('.trim-head .btn');
  await sleep(1500);
  const seek = async (t) => {
    await page.evaluate((t) => {
      const v = document.querySelector('.stage video');
      v.pause();
      v.currentTime = t;
    }, t);
    await sleep(500);
  };
  await seek(4);
  await page.keyboard.press('KeyI');
  await seek(17);
  await page.keyboard.press('KeyO');
  await seek(9.5);
  await page.evaluate(() => document.querySelectorAll('.lane')[1]?.focus());
  for (let i = 0; i < 7; i++) await page.keyboard.press('ArrowUp');
  await page.evaluate(() => document.activeElement?.blur());
  await sleep(900);
  await page.screenshot({ path: `${TMP}/${lang}-trim.png` });
}

// Link preview for Telegram and friends.
await page.setViewport({ width: 1200, height: 630, deviceScaleFactor: 1 });
await page.goto(`${SITE}/og.html`, { waitUntil: 'networkidle0' });
await sleep(600);
await page.screenshot({ path: 'site/img/og.png' });
await browser.close();

// PNG → WebP for the page.
execFileSync(
  'python',
  [
    '-c',
    `
import glob, os
from PIL import Image
for p in glob.glob('${TMP}/ru-*.png') + glob.glob('${TMP}/en-*.png'):
    name = os.path.splitext(os.path.basename(p))[0]
    Image.open(p).convert('RGB').save(f'site/img/screens/{name}.webp', 'WEBP', quality=84, method=6)
    print(name, os.path.getsize(f'site/img/screens/{name}.webp') // 1024, 'KB')
`,
  ],
  { stdio: 'inherit' },
);
