// Renders the website's app screenshots (one set per interface language) and
// the link-preview image with headless Edge. The app screens come from the
// UI's browser preview with its mock backend, so no real clips of anyone end
// up on the site.
//
//   npm run dev                                   (UI preview on :1420)
//   python -m http.server 8090 --directory site   (the site, for og.html)
//   node scripts/site-shots.mjs [lang ...]        (default: all 14)
//   node scripts/site-shots.mjs og                (only the link preview;
//                                                  no UI preview needed)
import puppeteer from 'puppeteer-core';
import { execFileSync } from 'node:child_process';
import { mkdirSync } from 'node:fs';

const EDGE = 'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe';
const APP = 'http://127.0.0.1:1420';
const SITE = 'http://127.0.0.1:8090';
const TMP = 'target/shots';
const ALL = ['ru', 'en', 'kk', 'uk', 'de', 'fr', 'es', 'pt-BR', 'pl', 'tr', 'it', 'zh-CN', 'ja', 'ko'];
const ARGS = process.argv.slice(2);
const LANGS = ARGS.length ? ARGS.filter((a) => a !== 'og') : ALL;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
mkdirSync(TMP, { recursive: true });

const browser = await puppeteer.launch({
  executablePath: EDGE,
  headless: true,
  args: ['--autoplay-policy=no-user-gesture-required', '--hide-scrollbars', '--mute-audio'],
});
const page = await browser.newPage();
await page.setViewport({ width: 1200, height: 780, deviceScaleFactor: 1.5 });

const done = [];
for (const lang of LANGS) {
  try {
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
    done.push(lang);
    console.log('shot', lang);
  } catch (e) {
    console.error(`${lang}: ${e.message}`);
  }
}

// Link preview for Telegram and friends.
try {
  await page.setViewport({ width: 1200, height: 630, deviceScaleFactor: 1 });
  await page.goto(`${SITE}/og.html`, { waitUntil: 'networkidle0' });
  await page.evaluate(() => document.fonts.ready);
  await sleep(600);
  await page.screenshot({ path: 'site/img/og.png' });
} catch (e) {
  console.error(`og.png: ${e.message}`);
}
await browser.close();

// PNG → WebP for the page.
execFileSync(
  'python',
  [
    '-c',
    `
import os
from PIL import Image
for lang in ${JSON.stringify(done)}:
    for shot in ('home', 'gallery', 'trim'):
        name = f'{lang}-{shot}'
        Image.open(f'${TMP}/{name}.png').convert('RGB').save(f'site/img/screens/{name}.webp', 'WEBP', quality=84, method=6)
        print(name, os.path.getsize(f'site/img/screens/{name}.webp') // 1024, 'KB')
`,
  ],
  { stdio: 'inherit' },
);
