import { cascade, pop, rise } from './motion';

/** Entrance for a list item: a just-saved clip pops in, others cascade
 *  (or just appear when `still`, for long lists). */
export function enter(node: Element, { i, fresh, still = false }: { i: number; fresh: boolean; still?: boolean }) {
  if (fresh) return pop(node);
  return still ? { duration: 0 } : rise(node, { delay: cascade(i) });
}
