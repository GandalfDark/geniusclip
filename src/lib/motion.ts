// Motion system: one soft curve and a few durations for the whole app.
// Keep in sync with the --dur / --ease tokens in app.css.
import { flip } from 'svelte/animate';

export const EASE = 'cubic-bezier(0.22, 1, 0.36, 1)';
export const DUR = 320;
export const DUR_FAST = 150;
export const STAGGER = 40;

/** Svelte easing close to EASE (ease-out quart). */
export const easeOut = (t: number) => 1 - Math.pow(1 - t, 4);

/** Honors Windows "Show animations" off / prefers-reduced-motion. */
export const reduced = () => typeof matchMedia !== 'undefined' && matchMedia('(prefers-reduced-motion: reduce)').matches;

/** Delay for the i-th item of a cascade (capped so long lists don't lag). */
export const cascade = (i: number) => Math.min(i, 10) * STAGGER;

type Opts = { delay?: number; duration?: number; y?: number; scale?: number };

/** Fade in while rising a few pixels — the default entrance. */
export function rise(_node: Element, { delay = 0, duration = DUR, y = 10, scale = 0.985 }: Opts = {}) {
  if (reduced()) return { duration: 0 };
  return {
    delay,
    duration,
    easing: easeOut,
    css: (t: number, u: number) => `opacity:${t};transform:translateY(${u * y}px) scale(${1 - (1 - scale) * u})`,
  };
}

/** Grow in from a smaller size — for a freshly saved clip. */
export function pop(_node: Element, { delay = 0, duration = DUR + 60 }: Opts = {}) {
  if (reduced()) return { duration: 0 };
  return {
    delay,
    duration,
    easing: easeOut,
    css: (t: number, u: number) => `opacity:${Math.min(1, t * 1.6)};transform:scale(${0.6 + 0.4 * t})`,
  };
}

/** Quick fade used for leaving elements, so exits never feel sluggish. */
export function leave(_node: Element, { duration = DUR_FAST }: Opts = {}) {
  if (reduced()) return { duration: 0 };
  return { duration, css: (t: number) => `opacity:${t}` };
}

export const flipParams = () => ({ duration: reduced() ? 0 : DUR, easing: easeOut });

/** Past this many items a list skips per-item entrances and FLIP moves. */
export const LONG_LIST = 40;

/** animate: FLIP move; `still` makes items jump into place (flip itself
 *  reads computed styles of every moved item). */
export function move(node: Element, fromTo: { from: DOMRect; to: DOMRect }, { still = false }: { still?: boolean } = {}) {
  return still ? { duration: 0 } : flip(node, fromTo, flipParams());
}

/** out: for list items. Leaving items stay in their container until they
 *  are gone, so it still counts the old list: a long one (e.g. filtered
 *  down to a few) drops them at once instead of fading dozens. */
export function leaveItem(node: Element) {
  return (node.parentElement?.childElementCount ?? 0) > LONG_LIST ? { duration: 0 } : leave(node);
}
