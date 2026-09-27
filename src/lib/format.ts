import { translate, type Lang } from './i18n';

// Intl formatters are costly to build and every card formats a date and a
// size, so each one is made once per language and options.
const formatters = new Map<string, unknown>();
function memo<T>(key: string, make: () => T): T {
  let f = formatters.get(key) as T | undefined;
  if (f === undefined) formatters.set(key, (f = make()));
  return f;
}
const numberFmt = (lang: Lang, digits: number) =>
  memo(`n:${lang}:${digits}`, () => new Intl.NumberFormat(lang, { minimumFractionDigits: digits, maximumFractionDigits: digits }));
const timeFmt = (lang: Lang) => memo(`t:${lang}`, () => new Intl.DateTimeFormat(lang, { hour: '2-digit', minute: '2-digit' }));
const dayFmt = (lang: Lang) => memo(`d:${lang}`, () => new Intl.DateTimeFormat(lang, { day: '2-digit', month: '2-digit' }));
type RelUnit = 'second' | 'minute' | 'hour' | 'day';
type Rel = { format(value: number, unit: RelUnit): string };
// WebView2's ICU has no Kazakh relative-time data and falls back to English
// ("today", "5 min ago"), so Kazakh gets its own few phrases.
const kkRel: Rel = {
  format(v, unit) {
    const n = Math.abs(v);
    if (unit === 'day') return v === 0 ? 'бүгін' : v === -1 ? 'кеше' : `${n} күн бұрын`;
    if (unit === 'second') return 'қазір';
    return unit === 'minute' ? `${n} мин бұрын` : `${n} сағ бұрын`;
  },
};
const relFmt = (lang: Lang, style: 'long' | 'short'): Rel =>
  lang === 'kk' && !Intl.RelativeTimeFormat.supportedLocalesOf('kk').length
    ? kkRel
    : memo(`r:${lang}:${style}`, () => new Intl.RelativeTimeFormat(lang, { numeric: 'auto', style }));

const UNITS = ['unit.b', 'unit.kb', 'unit.mb', 'unit.gb', 'unit.tb'] as const;

/** Timecode: 4:58, 12:03, 1:02:15. */
export function duration(sec: number): string {
  if (!isFinite(sec) || sec < 0) sec = 0;
  const s = Math.floor(sec % 60);
  const m = Math.floor(sec / 60) % 60;
  const h = Math.floor(sec / 3600);
  const mm = h ? String(m).padStart(2, '0') : String(m);
  return (h ? `${h}:` : '') + `${mm}:${String(s).padStart(2, '0')}`;
}

/** Timecode with tenths: 1:04.3 */
export function preciseTime(sec: number): string {
  // The epsilon keeps 2.3 (stored as 2.29999…) from showing as 2.2.
  return `${duration(sec)}.${Math.min(9, Math.floor((sec % 1) * 10 + 1e-6))}`;
}

/** "1,2 ГБ", "1.2 GB", "340 MB": units and decimal separator per language. */
export function bytes(n: number, lang: Lang): string {
  let i = 0;
  while (n >= 1024 && i < UNITS.length - 1) {
    n /= 1024;
    i++;
  }
  const v = numberFmt(lang, i >= 3 ? 1 : 0).format(n);
  return `${v} ${translate(lang, UNITS[i])}`;
}

/** "сегодня 21:10", "yesterday 21:10", "24.09 21:10" in the UI language. */
export function date(ms: number, lang: Lang): string {
  const d = new Date(ms);
  const now = new Date();
  const time = timeFmt(lang).format(d);
  const rel = relFmt(lang, 'long');
  if (d.toDateString() === now.toDateString()) return `${rel.format(0, 'day')} ${time}`;
  if (new Date(now.getTime() - 86400000).toDateString() === d.toDateString()) return `${rel.format(-1, 'day')} ${time}`;
  return `${dayFmt(lang).format(d)} ${time}`;
}

/** "2 min ago", "yesterday" in the UI language. */
export function ago(ms: number, lang: Lang): string {
  const rel = relFmt(lang, 'short');
  const s = (ms - Date.now()) / 1000;
  if (s > -45) return rel.format(0, 'second');
  if (s > -3600) return rel.format(Math.round(s / 60), 'minute');
  if (s > -86400) return rel.format(Math.round(s / 3600), 'hour');
  return rel.format(Math.round(s / 86400), 'day');
}

/** "Alt+Shift+KeyA" → ["Alt", "Shift", "A"] for display. */
const NUMPAD: Record<string, string> = { Add: '+', Subtract: '−', Multiply: '*', Divide: '/', Decimal: '.' };

export function hotkeyParts(accel: string): string[] {
  if (!accel) return [];
  return accel.split('+').map((p) =>
    p
      .replace(/^Key([A-Z])$/, '$1')
      .replace(/^Digit(\d)$/, '$1')
      .replace(/^Numpad(\d)$/, 'Num $1')
      .replace(/^Numpad(Add|Subtract|Multiply|Divide|Decimal)$/, (_, k: string) => `Num ${NUMPAD[k]}`)
      .replace(/^PageUp$/, 'PgUp')
      .replace(/^PageDown$/, 'PgDn')
      .replace(/^ScrollLock$/, 'Scroll Lock')
      .replace(/^PrintScreen$/, 'PrtSc')
      .replace(/^Control$/, 'Ctrl')
      .replace(/^Super$/, 'Win'),
  );
}

export type NotesBlock = { text: string } | { items: string[] };

/** Release notes as paragraphs and bullet lists: lines starting with "- "
 *  (or "* ", "• ") are list items; Markdown heading marks and bold are dropped. */
export function releaseNotes(notes: string): NotesBlock[] {
  const out: NotesBlock[] = [];
  for (const raw of notes.split(/\r?\n/)) {
    const line = raw.trim().replace(/\*\*|`/g, '');
    if (!line) continue;
    const item = /^[-*•]\s+(.*)$/.exec(line);
    const last = out[out.length - 1];
    if (!item) out.push({ text: line.replace(/^#+\s*/, '') });
    else if (last && 'items' in last) last.items.push(item[1]);
    else out.push({ items: [item[1]] });
  }
  return out;
}
