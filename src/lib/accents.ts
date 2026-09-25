// Accent presets: `color` is the solid accent, `color2` the end of the soft
// gradient used on the main button and switches. Keep `color` in sync with
// `accent()` in src-tauri/src/overlay.rs.
export const ACCENTS: Record<string, { color: string; color2: string; ru: string; en: string }> = {
  violet: { color: '#9580ff', color2: '#d77ce0', ru: 'Фиолетовый', en: 'Violet' },
  red: { color: '#ff5a36', color2: '#ff9a3c', ru: 'Красный', en: 'Red' },
  lime: { color: '#c6f432', color2: '#3ee0a0', ru: 'Лайм', en: 'Lime' },
  cyan: { color: '#38d2f0', color2: '#7c9dff', ru: 'Бирюзовый', en: 'Cyan' },
  amber: { color: '#ffb020', color2: '#ff6f61', ru: 'Янтарный', en: 'Amber' },
  mono: { color: '#e6e6ea', color2: '#a9a9b6', ru: 'Белый', en: 'White' },
};

export function applyAccent(id: string) {
  const acc = ACCENTS[id] ?? ACCENTS.violet;
  const root = document.documentElement.style;
  root.setProperty('--accent', acc.color);
  root.setProperty('--accent-2', acc.color2);
}
