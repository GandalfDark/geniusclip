import { cascade, pop, rise } from './motion';

/** Entrance for a list item: a just-saved clip pops in, others cascade. */
export function enter(node: Element, { i, fresh }: { i: number; fresh: boolean }) {
  return fresh ? pop(node) : rise(node, { delay: cascade(i) });
}
