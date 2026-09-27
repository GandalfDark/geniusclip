// GeniusClip download page: language, screenshots, version, star count and
// the release history from GitHub. No framework, no build step.
(() => {
  'use strict';

  const APP_REPO = 'GandalfDark/geniusclip';
  const RELEASES_REPO = 'GandalfDark/geniusclip-releases';
  const LANGS = {
    ru: 'Русский',
    en: 'English',
    kk: 'Қазақша',
    uk: 'Українська',
    de: 'Deutsch',
    fr: 'Français',
    es: 'Español',
    'pt-BR': 'Português (Brasil)',
    pl: 'Polski',
    tr: 'Türkçe',
    it: 'Italiano',
    'zh-CN': '简体中文',
    ja: '日本語',
    ko: '한국어',
  };

  const root = document.documentElement;
  const $ = (s, el = document) => el.querySelector(s);
  const $$ = (s, el = document) => [...el.querySelectorAll(s)];

  let lang = 'ru';
  let dict = {};
  const t = (k) => dict[k] ?? k;
  const store = {
    get: (k) => {
      try {
        return localStorage.getItem(k);
      } catch {
        return null;
      }
    },
    set: (k, v) => {
      try {
        localStorage.setItem(k, v);
      } catch {}
    },
  };

  /** Browser language → supported one (same mapping as the app). */
  function match(tag) {
    const l = String(tag || '').toLowerCase();
    if (l.startsWith('pt')) return 'pt-BR';
    if (l.startsWith('zh')) return 'zh-CN';
    const base = l.split('-')[0];
    if (base in LANGS) return base;
    if (['be', 'uz', 'ky', 'tg', 'tk', 'az', 'hy', 'ka'].includes(base)) return 'ru';
    return null;
  }

  function initialLang() {
    const q = new URLSearchParams(location.search).get('lang');
    if (q && q in LANGS) return q;
    const saved = store.get('lang');
    if (saved && saved in LANGS) return saved;
    for (const l of navigator.languages || [navigator.language]) {
      const m = match(l);
      if (m) return m;
    }
    return 'en';
  }

  // ------------------------------------------------------------ screenshots

  // One set per interface language; English if a file is missing.
  function showShots(l) {
    for (const img of $$('img[data-shot]')) {
      const want = `img/screens/${l}-${img.dataset.shot}.webp`;
      if (img.getAttribute('src') === want) continue;
      img.onerror = () => {
        img.onerror = null;
        img.src = `img/screens/en-${img.dataset.shot}.webp`;
      };
      img.src = want;
    }
    for (const a of $$('[data-shot-link]')) a.href = `img/screens/${l}-${a.dataset.shotLink}.webp`;
  }

  // ------------------------------------------------------------------ text

  // "Alt+F8" in a translation becomes <kbd>Alt</kbd>+<kbd>F8</kbd>.
  const KEYS = /((?:Alt|Ctrl|Strg|Shift)(?:\s?\+\s?(?:Alt|Ctrl|Strg|Shift|F\d{1,2}|[A-Z0-9]))+)(?![\w])/g;
  function setText(el, text) {
    if (!KEYS.test(text)) {
      el.textContent = text;
      return;
    }
    KEYS.lastIndex = 0;
    el.textContent = '';
    text.split(KEYS).forEach((part, i) => {
      if (i % 2 === 0) {
        if (part) el.append(part);
        return;
      }
      const combo = document.createElement('span');
      combo.className = 'keys';
      part.split(/\s?\+\s?/).forEach((key, j) => {
        if (j) combo.append('+');
        const kbd = document.createElement('kbd');
        kbd.textContent = key;
        combo.append(kbd);
      });
      el.append(combo);
    });
  }

  async function setLang(l) {
    try {
      const res = await fetch(`i18n/${l}.json`);
      if (!res.ok) throw new Error(res.status);
      dict = await res.json();
      lang = l;
    } catch {
      if (l !== 'ru') return setLang('ru');
    }
    root.lang = lang;
    document.title = t('meta.title');
    $('meta[name="description"]')?.setAttribute('content', t('meta.description'));
    for (const el of $$('[data-i18n]')) setText(el, t(el.dataset.i18n));
    for (const el of $$('[data-i18n-alt]')) el.alt = t(el.dataset.i18nAlt);
    for (const el of $$('[data-i18n-label]')) el.setAttribute('aria-label', t(el.dataset.i18nLabel));
    $('.lang-name').textContent = LANGS[lang];
    $('.lang-code').textContent = lang.split('-')[0].toUpperCase();
    $('.lang-btn').setAttribute('aria-label', `${t('lang.label')}: ${LANGS[lang]}`);
    $$('.lang-menu li').forEach((li) => li.setAttribute('aria-selected', String(li.dataset.lang === lang)));
    showShots(lang);
    renderMeta();
    renderStars();
    renderReleases();
  }

  // ---------------------------------------------------------- language menu

  const btn = $('.lang-btn');
  const menu = $('.lang-menu');
  menu.innerHTML = Object.entries(LANGS)
    .map(([code, name]) => `<li role="option" tabindex="-1" data-lang="${code}" lang="${code}">${name}</li>`)
    .join('');
  const openMenu = (open) => {
    menu.hidden = !open;
    btn.setAttribute('aria-expanded', String(open));
    if (open) ($('[aria-selected="true"]', menu) || menu.firstElementChild).focus();
  };
  btn.addEventListener('click', (e) => {
    e.stopPropagation();
    openMenu(menu.hidden);
  });
  menu.addEventListener('click', (e) => {
    const li = e.target.closest('li');
    if (!li) return;
    store.set('lang', li.dataset.lang);
    openMenu(false);
    btn.focus();
    setLang(li.dataset.lang);
  });
  menu.addEventListener('keydown', (e) => {
    const items = $$('li', menu);
    const i = items.indexOf(document.activeElement);
    if (e.key === 'ArrowDown') items[Math.min(i + 1, items.length - 1)].focus();
    else if (e.key === 'ArrowUp') items[Math.max(i - 1, 0)].focus();
    else if (e.key === 'Home') items[0].focus();
    else if (e.key === 'End') items[items.length - 1].focus();
    else if (e.key === 'Enter' || e.key === ' ') document.activeElement.click();
    else if (e.key === 'Tab') openMenu(false);
    else return;
    e.preventDefault();
  });
  document.addEventListener('click', () => openMenu(false));
  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && !menu.hidden) {
      openMenu(false);
      btn.focus();
    }
  });

  // ------------------------------------------------------------- GitHub API

  const gh = (path) => {
    const key = `gh:${path}`;
    try {
      const hit = JSON.parse(sessionStorage.getItem(key) || 'null');
      if (hit && Date.now() - hit.at < 10 * 60 * 1000) return Promise.resolve(hit.data);
    } catch {}
    return fetch(`https://api.github.com/repos/${path}`)
      .then((r) => (r.ok ? r.json() : Promise.reject(r.status)))
      .then((data) => {
        try {
          sessionStorage.setItem(key, JSON.stringify({ at: Date.now(), data }));
        } catch {}
        return data;
      });
  };
  const releases = gh(`${RELEASES_REPO}/releases?per_page=5`).catch(() => null);
  const repo = gh(APP_REPO).catch(() => null);

  const num = (n, opts) => new Intl.NumberFormat(lang, opts).format(n);

  async function renderMeta() {
    const list = (await releases) || [];
    const latest = list.find((r) => !r.draft && !r.prerelease);
    const asset = latest?.assets?.find((a) => a.name === 'GeniusClip-Setup.exe');
    if (!latest) return;
    const ver = $('.m-version');
    ver.textContent = latest.tag_name;
    ver.hidden = false;
    if (!asset) return;
    const size = $('.m-size');
    size.textContent = `${num(asset.size / 1048576, { maximumFractionDigits: 0 })} ${t('unit.mb')}`;
    size.hidden = false;
  }

  async function renderStars() {
    const r = await repo;
    const n = r?.stargazers_count;
    if (typeof n !== 'number') return;
    $('.stars-n').textContent = num(n, { notation: 'compact', maximumFractionDigits: 1 });
    $('.stars').hidden = false;
    $('.gh').setAttribute('aria-label', `GitHub, ${t('gh.stars')}: ${n}`);
  }

  const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c]);
  /** Release notes as written: plain lines and "- " bullets. */
  function notesHtml(body) {
    const out = [];
    let list = [];
    const flush = () => {
      if (list.length) out.push(`<ul>${list.map((x) => `<li>${esc(x)}</li>`).join('')}</ul>`);
      list = [];
    };
    for (const line of String(body || '').split(/\r?\n/)) {
      const m = line.match(/^\s*[-*•]\s+(.*)$/);
      if (m) list.push(m[1]);
      else {
        flush();
        if (line.trim()) out.push(`<p>${esc(line.trim().replace(/^#+\s*/, ''))}</p>`);
      }
    }
    flush();
    return out.join('');
  }

  async function renderReleases() {
    const all = await releases;
    const ol = $('.releases');
    if (!all) {
      ol.innerHTML = `<li class="rel-empty">${esc(t('news.failed'))}</li>`;
      return;
    }
    const list = all.filter((r) => !r.draft).slice(0, 5);
    if (!list.length) {
      ol.innerHTML = `<li class="rel-empty">${esc(t('news.empty'))}</li>`;
      return;
    }
    let date = new Intl.DateTimeFormat(lang, { day: 'numeric', month: 'long', year: 'numeric' });
    // Browsers without month names for a language print "2026 M09 27".
    if (/M\d/.test(date.format(new Date(2026, 8, 27)))) {
      date = new Intl.DateTimeFormat('ru', { day: '2-digit', month: '2-digit', year: 'numeric' });
    }
    ol.innerHTML = list
      .map((r) => {
        // Notes are written in English (older ones in Russian).
        const notesLang = /[А-Яа-яЁё]/.test(r.body || '') ? 'ru' : 'en';
        return `<li>
          <div class="rel-head">
            <span class="rel-version"><a href="${esc(r.html_url)}">${esc(r.tag_name)}</a></span>
            <time class="rel-date" datetime="${esc(r.published_at)}">${date.format(new Date(r.published_at))}</time>
          </div>
          <div class="rel-notes" lang="${notesLang}">${notesHtml(r.body)}</div>
        </li>`;
      })
      .join('');
  }

  // ---------------------------------------------------------- odds and ends

  // Phones and Macs can't run it: say so, and offer to copy the link.
  const ua = navigator.userAgent;
  const onWindows = /Windows/i.test(ua) && !/Phone|Mobile/i.test(ua);
  if (!onWindows) {
    $('.mobile-note').hidden = false;
    $('.copy-link').addEventListener('click', async (e) => {
      const b = e.currentTarget;
      try {
        await navigator.clipboard.writeText(location.href.split('#')[0]);
        b.textContent = t('dl.copied');
      } catch {}
    });
  }

  const first = initialLang();
  showShots(first);
  setLang(first);
})();
