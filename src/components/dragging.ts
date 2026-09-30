// Pointer-driven drag for virtualized tables. Targets are absolute slots/gaps computed
// from the pointer position, never indices into a filtered array.
import { useApp } from "../stores/app";
import { useDrag, type DragTarget } from "../stores/drag";

interface DropZone {
  el: HTMLElement;
  rowH: number;
  /** Absolute slot for a displayed row index (handles changed-only filtering). */
  slotAt: (index: number) => number | null;
  count: () => number;
}

let zone: DropZone | null = null;
export function registerBankDropZone(z: DropZone | null) {
  zone = z;
}

const THRESHOLD = 4;

function computeTarget(x: number, y: number): DragTarget {
  const d = useDrag.getState();
  if (!zone) return null;
  const r = zone.el.getBoundingClientRect();
  if (x < r.left || x > r.right || y < r.top || y > r.bottom) return null;
  const pos = y - r.top + zone.el.scrollTop;
  const n = zone.count();
  if (n === 0) return null;
  let idx = Math.floor(pos / zone.rowH);
  const within = pos - idx * zone.rowH;
  if (idx >= n) {
    idx = n - 1;
  }
  const slot = zone.slotAt(idx);
  if (slot == null) return null;
  if (d.source === "library") return { type: "replace", slot };
  return { type: "gap", gap: within < zone.rowH / 2 ? slot : slot + 1 };
}

export function beginPointerDrag(e: React.MouseEvent, payload: { source: "library" | "bank"; libIds?: string[]; bankSlots?: number[] }, onClick: () => void) {
  const sx = e.clientX;
  const sy = e.clientY;
  let started = false;
  let scrollTimer: number | null = null;
  let lastY = sy;

  const autoscroll = () => {
    if (!zone) return;
    const r = zone.el.getBoundingClientRect();
    const edge = 36;
    if (lastY < r.top + edge && lastY > r.top - 60) zone.el.scrollTop -= Math.max(4, (r.top + edge - lastY) / 2);
    else if (lastY > r.bottom - edge && lastY < r.bottom + 60) zone.el.scrollTop += Math.max(4, (lastY - (r.bottom - edge)) / 2);
  };

  const move = (ev: MouseEvent) => {
    lastY = ev.clientY;
    if (!started) {
      if (Math.abs(ev.clientX - sx) + Math.abs(ev.clientY - sy) < THRESHOLD) return;
      started = true;
      useDrag.getState().set({ active: true, source: payload.source, libIds: payload.libIds ?? [], bankSlots: payload.bankSlots ?? [] });
      scrollTimer = window.setInterval(() => {
        autoscroll();
        const st = useDrag.getState();
        if (st.active) st.set({ target: computeTarget(st.x, lastY) });
      }, 50);
    }
    useDrag.getState().set({ x: ev.clientX, y: ev.clientY, target: computeTarget(ev.clientX, ev.clientY) });
  };

  const cleanup = () => {
    window.removeEventListener("mousemove", move);
    window.removeEventListener("mouseup", up);
    window.removeEventListener("keydown", key, true);
    if (scrollTimer) clearInterval(scrollTimer);
  };

  const key = (ev: KeyboardEvent) => {
    if (ev.key === "Escape" && started) {
      ev.preventDefault();
      ev.stopPropagation();
      cleanup();
      useDrag.getState().reset();
    }
  };

  const up = () => {
    cleanup();
    if (!started) {
      onClick();
      return;
    }
    const d = useDrag.getState();
    const t = d.target;
    const libIds = d.libIds;
    const bankSlots = d.bankSlots;
    const source = d.source;
    d.reset();
    if (!t) return;
    const app = useApp.getState();
    if (source === "library" && t.type === "replace") void app.applyOp({ type: "ReplaceFromLibrary", start: t.slot, occurrence_ids: libIds });
    else if (source === "bank" && t.type === "gap") void app.applyOp({ type: "MoveToGap", slots: bankSlots, gap: t.gap });
  };

  window.addEventListener("mousemove", move);
  window.addEventListener("mouseup", up);
  window.addEventListener("keydown", key, true);
}
