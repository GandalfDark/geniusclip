// Accent presets: `color` is the solid accent, `color2` the end of the soft
// gradient used on the main button and switches. Keep `color` in sync with
// `accent()` in src-tauri/src/overlay.rs.
// Names are the `accent.<id>` strings in the locale files.
export const ACCENTS = {
  violet: { color: '#9580ff', color2: '#d77ce0' },
  red: { color: '#ff5a36', color2: '#ff9a3c' },
  lime: { color: '#c6f432', color2: '#3ee0a0' },
  cyan: { color: '#38d2f0', color2: '#7c9dff' },
  amber: { color: '#ffb020', color2: '#ff6f61' },
  mono: { color: '#e6e6ea', color2: '#a9a9b6' },
} as const;
export type AccentId = keyof typeof ACCENTS;

export function applyAccent(id: string) {
  const acc = ACCENTS[id as AccentId] ?? ACCENTS.violet;
  const root = document.documentElement.style;
  root.setProperty('--accent', acc.color);
  root.setProperty('--accent-2', acc.color2);
}
