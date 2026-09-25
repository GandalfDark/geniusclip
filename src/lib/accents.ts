// Flat accent presets. Keep in sync with `accent()` in src-tauri/src/overlay.rs.
export const ACCENTS: Record<string, { color: string; ru: string; en: string }> = {
  violet: { color: '#9580ff', ru: 'Фиолетовый', en: 'Violet' },
  red: { color: '#ff5a36', ru: 'Красный', en: 'Red' },
  lime: { color: '#c6f432', ru: 'Лайм', en: 'Lime' },
  cyan: { color: '#38d2f0', ru: 'Бирюзовый', en: 'Cyan' },
  amber: { color: '#ffb020', ru: 'Янтарный', en: 'Amber' },
  mono: { color: '#e6e6ea', ru: 'Белый', en: 'White' },
};

export function applyAccent(id: string) {
  const acc = ACCENTS[id] ?? ACCENTS.violet;
  document.documentElement.style.setProperty('--accent', acc.color);
}
