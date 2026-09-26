import { translate, type Lang } from './i18n';

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
  return `${duration(sec)}.${Math.floor((sec % 1) * 10)}`;
}

/** "1,2 ГБ", "1.2 GB", "340 MB": units and decimal separator per language. */
export function bytes(n: number, lang: Lang): string {
  const units = (['unit.b', 'unit.kb', 'unit.mb', 'unit.gb', 'unit.tb'] as const).map((k) => translate(lang, k));
  let i = 0;
  while (n >= 1024 && i < units.length - 1) {
    n /= 1024;
    i++;
  }
  const digits = i >= 3 ? 1 : 0;
  const v = new Intl.NumberFormat(lang, { minimumFractionDigits: digits, maximumFractionDigits: digits }).format(n);
  return `${v} ${units[i]}`;
}

/** "сегодня 21:10", "yesterday 21:10", "24.09 21:10" in the UI language. */
export function date(ms: number, lang: Lang): string {
  const d = new Date(ms);
  const now = new Date();
  const time = d.toLocaleTimeString(lang, { hour: '2-digit', minute: '2-digit' });
  const rel = new Intl.RelativeTimeFormat(lang, { numeric: 'auto' });
  if (d.toDateString() === now.toDateString()) return `${rel.format(0, 'day')} ${time}`;
  if (new Date(now.getTime() - 86400000).toDateString() === d.toDateString()) return `${rel.format(-1, 'day')} ${time}`;
  return `${d.toLocaleDateString(lang, { day: '2-digit', month: '2-digit' })} ${time}`;
}

/** "2 min ago", "yesterday" in the UI language. */
export function ago(ms: number, lang: Lang): string {
  const rel = new Intl.RelativeTimeFormat(lang, { numeric: 'auto', style: 'short' });
  const s = (ms - Date.now()) / 1000;
  if (s > -45) return rel.format(0, 'second');
  if (s > -3600) return rel.format(Math.round(s / 60), 'minute');
  if (s > -86400) return rel.format(Math.round(s / 3600), 'hour');
  return rel.format(Math.round(s / 86400), 'day');
}

/** "Alt+Shift+KeyA" → ["Alt", "Shift", "A"] for display. */
export function hotkeyParts(accel: string): string[] {
  if (!accel) return [];
  return accel.split('+').map((p) =>
    p
      .replace(/^Key([A-Z])$/, '$1')
      .replace(/^Digit(\d)$/, '$1')
      .replace(/^Control$/, 'Ctrl')
      .replace(/^Super$/, 'Win'),
  );
}
