// Keyboard over a card's list: Ctrl+↓ Ctrl+↑ highlight a row, Ctrl+O does
// what a click on it does (the Mac's cardSelection). A list is any element
// with the `card-list` class; each row registers what opening it does.

const rowOpens = new WeakMap<Element, () => void>();

/** Registers what Ctrl+O on this row does (the same as clicking it). */
export function onRowOpen(el: Element, open: () => void) {
  rowOpens.set(el, open);
}

/** The rows of the list shown in `root`, top to bottom; none without one. */
export function listRows(root: HTMLElement): HTMLElement[] {
  const list = root.querySelector(".card-list");
  return list ? (Array.from(list.children) as HTMLElement[]) : [];
}

/**
 * Highlights row `selection` and no other. `reveal`: the highlight just
 * moved, so its row scrolls into view.
 */
export function highlightRow(rows: readonly HTMLElement[], selection: number | null, reveal: boolean) {
  rows.forEach((el, i) => el.classList.toggle("selected", i === selection));
  if (reveal && selection != null) rows[selection]?.scrollIntoView({ block: "nearest" });
}

/** Ctrl+O: does what a click on the row does. */
export function openRow(el: HTMLElement | undefined) {
  if (el) rowOpens.get(el)?.();
}
