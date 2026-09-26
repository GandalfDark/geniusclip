// GeniusClip download page: language, screenshots, live version info and
// the release history from GitHub. No framework, no build step.
(() => {
  'use strict';

  const REPO = 'GandalfDark/geniusclip-releases';
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
  // Screenshots exist in Russian and English.
  const SHOT_LANG = (l) => (['ru', 'uk', 'kk'].includes(l) ? 'ru' : 'en');

  const root = document.documentElement;
  root.classList.add('js');
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
    for (const el of $$('[data-i18n]')) el.textContent = t(el.dataset.i18n);
    $('.lang-name').textContent = LANGS[lang];
    $$('.lang-menu li').forEach((li) => li.setAttribute('aria-selected', String(li.dataset.lang === lang)));
    for (const img of $$('.shots img')) img.src = `img/screens/${SHOT_LANG(lang)}-${img.dataset.shot}.webp`;
    const active = $('.tabs [aria-selected="true"]');
    $('.shot-caption').textContent = t(`screens.${active.dataset.shot}Text`);
    requestAnimationFrame(movePill);
    renderMeta();
    renderReleases();
  }

  // ---------------------------------------------------------- language menu

  const btn = $('.lang-btn');
  const menu = $('.lang-menu');
  menu.innerHTML = Object.entries(LANGS)
    .map(([code, name]) => `<li role="option" tabindex="0" data-lang="${code}" lang="${code}">${name}</li>`)
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
    setLang(li.dataset.lang);
  });
  menu.addEventListener('keydown', (e) => {
    const items = $$('li', menu);
    const i = items.indexOf(document.activeElement);
    if (e.key === 'ArrowDown') items[Math.min(i + 1, items.length - 1)].focus();
    else if (e.key === 'ArrowUp') items[Math.max(i - 1, 0)].focus();
    else if (e.key === 'Enter') document.activeElement.click();
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

  // ------------------------------------------------------------ screenshots

  const tabs = $('.tabs');
  function movePill() {
    const active = $('[aria-selected="true"]', tabs);
    const pill = $('.tab-pill', tabs);
    pill.style.width = `${active.offsetWidth}px`;
    pill.style.transform = `translateX(${active.offsetLeft}px)`;
  }
  tabs.addEventListener('click', (e) => {
    const b = e.target.closest('button');
    if (!b) return;
    $$('button', tabs).forEach((x) => x.setAttribute('aria-selected', String(x === b)));
    $$('.shots img').forEach((img) => img.classList.toggle('active', img.dataset.shot === b.dataset.shot));
    $('.shot-caption').textContent = t(`screens.${b.dataset.shot}Text`);
    movePill();
  });
  addEventListener('resize', movePill);

  // --------------------------------------------------- version and history

  const gh = (path) => {
    const key = `gh:${path}`;
    try {
      const hit = JSON.parse(sessionStorage.getItem(key) || 'null');
      if (hit && Date.now() - hit.at < 10 * 60 * 1000) return Promise.resolve(hit.data);
    } catch {}
    return fetch(`https://api.github.com/repos/${REPO}/${path}`)
      .then((r) => (r.ok ? r.json() : Promise.reject(r.status)))
      .then((data) => {
        try {
          sessionStorage.setItem(key, JSON.stringify({ at: Date.now(), data }));
        } catch {}
        return data;
      });
  };
  const releases = gh('releases?per_page=5').catch(() => []);

  async function renderMeta() {
    const list = await releases;
    const latest = list.find((r) => !r.draft && !r.prerelease);
    const asset = latest?.assets?.find((a) => a.name === 'GeniusClip-Setup.exe');
    if (!latest || !asset) return;
    const mb = new Intl.NumberFormat(lang, { maximumFractionDigits: 0 }).format(asset.size / 1048576);
    $('.dl-version').textContent = ` · ${latest.tag_name} · ${mb} ${t('unit.mb')}`;
  }

  const esc = (s) => s.replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c]);
  /** Release notes: plain lines and "- " bullets. */
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
    const list = (await releases).filter((r) => !r.draft);
    if (!list.length) return;
    const date = new Intl.DateTimeFormat(lang, { day: 'numeric', month: 'long', year: 'numeric' });
    $('.releases').innerHTML = list
      .map(
        (r) => `<li>
          <div class="rel-head"><span class="rel-version">${esc(r.tag_name)}</span><span class="rel-date">${date.format(new Date(r.published_at))}</span></div>
          <div class="rel-notes">${notesHtml(r.body)}</div>
        </li>`,
      )
      .join('');
  }

  // ---------------------------------------------------------- replay tape

  // Abstract "game frames": sky, horizon glow and ground, in game-like palettes.
  const palettes = [
    ['#3b1d6e', '#ff6b9d', '#1a1030'],
    ['#0e3a5c', '#22d3ee', '#0a1f2e'],
    ['#5c2a0e', '#fbbf24', '#2a1608'],
    ['#123d2b', '#34d399', '#0a2018'],
    ['#2b1a5c', '#a78bfa', '#140e2e'],
    ['#5c1a1a', '#fb7185', '#2a0c0c'],
    ['#1d2a5c', '#60a5fa', '#0c142e'],
    ['#4a3a0e', '#facc15', '#221a06'],
  ];
  const frames = $('.frames');
  const tile = (i) => {
    const [sky, glow, ground] = palettes[i % palettes.length];
    const h = 38 + ((i * 17) % 26);
    return `<i style="background:
      radial-gradient(60px 26px at ${20 + ((i * 29) % 70)}% ${h}%, ${glow}cc, transparent 70%),
      linear-gradient(180deg, ${sky} 0%, ${sky} ${h - 6}%, ${glow}55 ${h}%, ${ground} ${h + 4}%, ${ground} 100%)"></i>`;
  };
  const set = Array.from({ length: 14 }, (_, i) => tile(i)).join('');
  frames.innerHTML = set + set;

  // ---------------------------------------------------------- odds and ends

  // Phones and Macs can't run it: say so, and offer to copy the link.
  const ua = navigator.userAgent;
  const onWindows = /Windows/i.test(ua) && !/Phone|Mobile/i.test(ua);
  if (!onWindows) {
    $('.mobile-note').hidden = false;
    $('.copy-link').addEventListener('click', async (e) => {
      try {
        await navigator.clipboard.writeText(location.href.split('#')[0]);
        e.currentTarget.textContent = t('dl.copied');
      } catch {}
    });
  }

  const nav = $('.nav');
  const onScroll = () => nav.classList.toggle('scrolled', scrollY > 8);
  addEventListener('scroll', onScroll, { passive: true });
  onScroll();

  // Reveal on scroll, cascading within a group.
  const io = new IntersectionObserver(
    (entries) => {
      for (const e of entries) {
        if (!e.isIntersecting) continue;
        const siblings = [...e.target.parentElement.children].filter((x) => x.classList.contains('reveal'));
        e.target.style.transitionDelay = `${Math.min(siblings.indexOf(e.target), 6) * 60}ms`;
        e.target.classList.add('in');
        io.unobserve(e.target);
      }
    },
    { rootMargin: '0px 0px -8% 0px' },
  );
  $$('.reveal').forEach((el) => io.observe(el));

  setLang(initialLang());
})();
