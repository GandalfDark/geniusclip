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

export function bytes(n: number, lang: 'ru' | 'en'): string {
  const units = lang === 'ru' ? ['Б', 'КБ', 'МБ', 'ГБ', 'ТБ'] : ['B', 'KB', 'MB', 'GB', 'TB'];
  let i = 0;
  while (n >= 1024 && i < units.length - 1) {
    n /= 1024;
    i++;
  }
  const v = i >= 3 ? n.toFixed(1) : Math.round(n).toString();
  return `${lang === 'ru' ? v.replace('.', ',') : v} ${units[i]}`;
}

/** "сегодня 21:10", "вчера 21:10", "24.09 21:10". */
export function date(ms: number, lang: 'ru' | 'en'): string {
  const d = new Date(ms);
  const now = new Date();
  const time = d.toLocaleTimeString(lang, { hour: '2-digit', minute: '2-digit' });
  if (d.toDateString() === now.toDateString()) return `${lang === 'ru' ? 'сегодня' : 'today'} ${time}`;
  if (new Date(now.getTime() - 86400000).toDateString() === d.toDateString()) return `${lang === 'ru' ? 'вчера' : 'yesterday'} ${time}`;
  const dd = `${String(d.getDate()).padStart(2, '0')}.${String(d.getMonth() + 1).padStart(2, '0')}`;
  return `${dd} ${time}`;
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
