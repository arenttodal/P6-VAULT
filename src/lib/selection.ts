// Selection by stable IDs. Order for operations comes from the current displayed order.

export interface Selection<K> {
  ids: Set<K>;
  anchor: K | null;
  focus: K | null;
}

export function emptySelection<K>(): Selection<K> {
  return { ids: new Set(), anchor: null, focus: null };
}

export function clickSelect<K>(sel: Selection<K>, order: K[], id: K, mods: { meta: boolean; shift: boolean }): Selection<K> {
  if (mods.shift && sel.anchor != null) {
    const a = order.indexOf(sel.anchor);
    const b = order.indexOf(id);
    if (a >= 0 && b >= 0) {
      const [lo, hi] = a < b ? [a, b] : [b, a];
      const ids = new Set(mods.meta ? sel.ids : []);
      for (let i = lo; i <= hi; i++) ids.add(order[i]);
      return { ids, anchor: sel.anchor, focus: id };
    }
  }
  if (mods.meta) {
    const ids = new Set(sel.ids);
    if (ids.has(id)) ids.delete(id);
    else ids.add(id);
    return { ids, anchor: id, focus: id };
  }
  return { ids: new Set([id]), anchor: id, focus: id };
}

export function selectAll<K>(order: K[], focus: K | null): Selection<K> {
  return { ids: new Set(order), anchor: order[0] ?? null, focus: focus ?? order[0] ?? null };
}

/** Drop selected ids no longer visible (after filtering). Returns the pruned selection. */
export function pruneToVisible<K>(sel: Selection<K>, order: K[]): Selection<K> {
  const visible = new Set(order);
  const ids = new Set([...sel.ids].filter((i) => visible.has(i)));
  if (ids.size === sel.ids.size) return sel;
  return {
    ids,
    anchor: sel.anchor != null && visible.has(sel.anchor) ? sel.anchor : null,
    focus: sel.focus != null && visible.has(sel.focus) ? sel.focus : null,
  };
}

/** Selected ids frozen in display order. */
export function orderedSelection<K>(sel: Selection<K>, order: K[]): K[] {
  return order.filter((k) => sel.ids.has(k));
}

export function moveFocus<K>(sel: Selection<K>, order: K[], delta: number, extend: boolean): Selection<K> {
  if (order.length === 0) return sel;
  const cur = sel.focus != null ? order.indexOf(sel.focus) : -1;
  const next = order[Math.max(0, Math.min(order.length - 1, cur < 0 ? 0 : cur + delta))];
  if (extend) return clickSelect({ ...sel, anchor: sel.anchor ?? sel.focus ?? next }, order, next, { meta: false, shift: true });
  return { ids: new Set([next]), anchor: next, focus: next };
}

/** Whether an unmodified shortcut should be ignored because the user is typing. */
export function isTypingTarget(el: EventTarget | null): boolean {
  if (!(el instanceof HTMLElement)) return false;
  if (el.isContentEditable) return true;
  const tag = el.tagName;
  if (tag === "TEXTAREA" || tag === "SELECT") return true;
  if (tag === "INPUT") {
    const t = (el as HTMLInputElement).type;
    return !["checkbox", "radio", "button", "range"].includes(t);
  }
  return false;
}
