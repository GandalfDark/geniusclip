export const ACCENTS: Record<string, { a: string; b: string; ru: string; en: string }> = {
  aurora: { a: '#f472b6', b: '#a78bfa', ru: 'Аврора', en: 'Aurora' },
  lagoon: { a: '#22d3ee', b: '#818cf8', ru: 'Лагуна', en: 'Lagoon' },
  ember: { a: '#ff5a6e', b: '#ff9f43', ru: 'Пламя', en: 'Ember' },
  toxic: { a: '#a3e635', b: '#2dd4bf', ru: 'Токсик', en: 'Toxic' },
  gold: { a: '#fcd34d', b: '#f472b6', ru: 'Закат', en: 'Sunset' },
  frost: { a: '#c7d2fe', b: '#60a5fa', ru: 'Иней', en: 'Frost' },
};

export function applyAccent(id: string) {
  const acc = ACCENTS[id] ?? ACCENTS.aurora;
  const root = document.documentElement.style;
  root.setProperty('--accent-a', acc.a);
  root.setProperty('--accent-b', acc.b);
}
