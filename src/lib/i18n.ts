// UI translations. Each language lives in src/lib/locales/<code>.ts; Russian
// is the source and defines the key set. The backend's own strings (tray,
// overlay) are in src-tauri/locales/<code>.json.
import ru, { strings as ruStrings } from './locales/ru';
import en from './locales/en';
import kk from './locales/kk';
import uk from './locales/uk';
import de from './locales/de';
import fr from './locales/fr';
import es from './locales/es';
import ptBR from './locales/pt-BR';
import pl from './locales/pl';
import tr from './locales/tr';
import it from './locales/it';
import zhCN from './locales/zh-CN';
import ja from './locales/ja';
import ko from './locales/ko';

export type TKey = keyof typeof ruStrings;

export const isKey = (k: string): k is TKey => k in ruStrings;

/** Plural forms keyed by Intl.PluralRules category; `{n}` is the number.
 *  `exact1` (optional) is used for exactly 1 ("the last minute"). */
export type Plural = { exact1?: string; zero?: string; one?: string; two?: string; few?: string; many?: string; other: string };

export interface Locale {
  /** Name of the language in itself ("Deutsch"). */
  name: string;
  strings: Record<TKey, string>;
  /** Home status line: "The last N minutes are always ready". */
  readyMin: Plural;
  /** Same for buffers shorter than a minute. */
  readySec: Plural;
}

export const LOCALES = { ru, en, kk, uk, de, fr, es, 'pt-BR': ptBR, pl, tr, it, 'zh-CN': zhCN, ja, ko } satisfies Record<string, Locale>;
export type Lang = keyof typeof LOCALES;
export const LANGS = Object.keys(LOCALES) as Lang[];

export function isLang(l: string | null | undefined): l is Lang {
  return !!l && l in LOCALES;
}

/** Best match for a BCP 47 tag ("de-AT" → de, "zh-TW" → zh-CN, "be" → ru). */
export function matchLang(tag: string): Lang {
  const t = tag.toLowerCase();
  if (t.startsWith('pt')) return 'pt-BR';
  if (t.startsWith('zh')) return 'zh-CN';
  const base = t.split('-')[0];
  if (isLang(base)) return base;
  // Other CIS languages: Russian is the most familiar second language.
  if (['be', 'uz', 'ky', 'tg', 'tk', 'az', 'hy', 'ka'].includes(base)) return 'ru';
  return 'en';
}

export function translate(lang: Lang, key: TKey, vars?: Record<string, string | number>): string {
  let s = LOCALES[lang].strings[key] ?? LOCALES.en.strings[key] ?? key;
  if (vars) for (const [k, v] of Object.entries(vars)) s = s.replaceAll(`{${k}}`, String(v));
  return s;
}

function plural(lang: Lang, forms: Plural, n: number): string {
  const tpl = (n === 1 && forms.exact1) || forms[new Intl.PluralRules(lang).select(n)] || forms.other;
  return tpl.replace('{n}', new Intl.NumberFormat(lang, { maximumFractionDigits: 1 }).format(n));
}

/** "The last 2 minutes are always ready", with the language's plural rules. */
export function readyLine(lang: Lang, seconds: number): string {
  const l = LOCALES[lang];
  return seconds < 60 ? plural(lang, l.readySec, seconds) : plural(lang, l.readyMin, seconds / 60);
}
