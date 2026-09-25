export function duration(sec: number): string {
  if (!isFinite(sec) || sec < 0) sec = 0;
  const s = Math.floor(sec % 60);
  const m = Math.floor(sec / 60) % 60;
  const h = Math.floor(sec / 3600);
  const mm = h ? String(m).padStart(2, '0') : String(m);
  return (h ? `${h}:` : '') + `${mm}:${String(s).padStart(2, '0')}`;
}

export function preciseTime(sec: number): string {
  const whole = duration(sec);
  const tenth = Math.floor((sec % 1) * 10);
  return `${whole}.${tenth}`;
}

export function bytes(n: number, lang: 'ru' | 'en'): string {
  const units = lang === 'ru' ? ['Б', 'КБ', 'МБ', 'ГБ', 'ТБ'] : ['B', 'KB', 'MB', 'GB', 'TB'];
  let i = 0;
  while (n >= 1024 && i < units.length - 1) {
    n /= 1024;
    i++;
  }
  const v = i >= 2 ? n.toFixed(n < 10 ? 1 : 0) : Math.round(n).toString();
  return `${lang === 'ru' ? v.replace('.', ',') : v} ${units[i]}`;
}

export function date(ms: number, lang: 'ru' | 'en'): string {
  const d = new Date(ms);
  const now = new Date();
  const time = d.toLocaleTimeString(lang, { hour: '2-digit', minute: '2-digit' });
  const sameDay = d.toDateString() === now.toDateString();
  const yesterday = new Date(now.getTime() - 86400000).toDateString() === d.toDateString();
  if (sameDay) return (lang === 'ru' ? 'Сегодня, ' : 'Today, ') + time;
  if (yesterday) return (lang === 'ru' ? 'Вчера, ' : 'Yesterday, ') + time;
  return d.toLocaleDateString(lang, { day: 'numeric', month: 'short' }) + ', ' + time;
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
