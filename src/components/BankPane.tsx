import { useVirtualizer } from "@tanstack/react-virtual";
import { useEffect, useMemo, useRef } from "react";
import { api } from "../api/backend";
import type { SlotView } from "../api/types";
import { slot3 } from "../lib/format";
import { clickSelect, orderedSelection, pruneToVisible } from "../lib/selection";
import { useApp } from "../stores/app";
import { useDrag } from "../stores/drag";
import { CategoryMenu, CategoryTag } from "./common";
import { beginPointerDrag, registerBankDropZone } from "./dragging";

export const BANK_ROW_H = 24;

export function useVisibleSlots(): SlotView[] {
  const ws = useApp((s) => s.workspace);
  const changedOnly = useApp((s) => s.changedOnly);
  return useMemo(() => (ws ? (changedOnly ? ws.slots.filter((s) => s.changed || !s.new) : ws.slots) : []), [ws, changedOnly]);
}

export function BankPane() {
  const { workspace, bankSel, set, pane, changedOnly, applyOp, applyMeta, openDialog, toast, error, clipboardCount } = useApp();
  const slots = useVisibleSlots();
  const order = useMemo(() => slots.map((s) => s.slot), [slots]);
  const scrollRef = useRef<HTMLDivElement>(null);
  const v = useVirtualizer({ count: slots.length, getScrollElement: () => scrollRef.current, estimateSize: () => BANK_ROW_H, overscan: 20 });
  const drag = useDrag();

  useEffect(() => {
    if (!scrollRef.current) return;
    registerBankDropZone({ el: scrollRef.current, rowH: BANK_ROW_H, slotAt: (i) => order[i] ?? null, count: () => order.length });
    return () => registerBankDropZone(null);
  }, [order]);

  useEffect(() => {
    const pruned = pruneToVisible(bankSel, order);
    if (pruned !== bankSel) set({ bankSel: pruned });
  }, [order]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    if (bankSel.focus != null) {
      const i = order.indexOf(bankSel.focus);
      if (i >= 0) v.scrollToIndex(i, { align: "auto" });
    }
  }, [bankSel.focus]); // eslint-disable-line react-hooks/exhaustive-deps

  if (!workspace) {
    return (
      <section className="pane bank">
        <div className="pane-head">
          <h2>Bank</h2>
        </div>
        <div className="empty big">
          <p>No bank yet.</p>
          <p>Connect and choose <b>Sync from P6</b> to capture Current (000–499), or import a full-bank .syx and click <b>Bank</b> next to it in Sources to organize offline.</p>
        </div>
      </section>
    );
  }

  const selected = orderedSelection(bankSel, order).sort((a, b) => a - b);
  const occIds = selected.map((s) => workspace.slots[s].new?.occurrence_id).filter((x): x is string => !!x);
  const t = drag.active && drag.target;
  const replaceRange =
    t && t.type === "replace" ? { from: t.slot, to: t.slot + drag.libIds.length - 1, overflow: t.slot + drag.libIds.length > 500 } : null;

  const copySel = async () => {
    const items = selected.map((s) => workspace.slots[s].new).filter((c) => !!c).map((c) => ({ blob_hash: c!.blob_hash, occurrence_id: c!.occurrence_id }));
    try {
      const n = await api().copyPrograms(items);
      set({ clipboardCount: n });
      toast("info", `Copied ${n} program(s).`);
    } catch (e) {
      error(e);
    }
  };

  return (
    <section className={`pane bank ${pane === "bank" ? "active-pane" : ""}`} onMouseDown={() => pane !== "bank" && set({ pane: "bank" })}>
      <div className="pane-head">
        <h2>{workspace.name}</h2>
        <span className="muted small">
          Current: {workspace.baseline ? `${workspace.baseline.kind === "imported" ? "imported" : "captured"} ${new Date(workspace.baseline.captured_end_ms).toLocaleString()}` : "none (offline bank)"}
          {workspace.empty_count > 0 && <b className="warn"> · {workspace.empty_count} empty</b>}
        </span>
        <label className="toggle">
          <input type="checkbox" checked={changedOnly} onChange={(e) => set({ changedOnly: e.target.checked })} /> Changed only ({workspace.changed_count})
        </label>
      </div>
      <div className="pane-tools">
        <span className="selcount">{selected.length ? `${selected.length} selected` : "500 slots"}</span>
        <button disabled={!selected.length} onClick={() => openDialog({ kind: "moveTo", slots: selected })}>
          Move to…
        </button>
        <button disabled={!selected.length} onClick={() => openDialog({ kind: "copyTo", slots: selected })}>
          Copy to…
        </button>
        <button disabled={!selected.length} onClick={() => openDialog({ kind: "swap", slots: selected })}>
          Swap…
        </button>
        <button disabled={selected.length < 2} onClick={() => void applyOp({ type: "SortSelected", slots: selected, key: "Name" })}>
          Sort A–Z
        </button>
        <button disabled={selected.length < 2} onClick={() => void applyOp({ type: "SortSelected", slots: selected, key: "Category" })}>
          Group by category
        </button>
        <button disabled={!selected.length} onClick={() => void copySel()}>
          Copy
        </button>
        <button disabled={!clipboardCount || bankSel.focus == null} onClick={() => void applyOp({ type: "Paste", start: bankSel.focus ?? 0, clipboard: "" })}>
          Paste
        </button>
        <CategoryMenu disabled={!occIds.length} onPick={(c) => void applyMeta({ type: "SetCategory", occurrence_ids: occIds, category: c })} />
        <button disabled={!selected.length || !workspace.baseline} onClick={() => void applyOp({ type: "RevertSelected", slots: selected })}>
          Revert to Current
        </button>
        <button
          disabled={!workspace.baseline || workspace.changed_count === 0}
          onClick={() =>
            openDialog({
              kind: "confirm",
              title: "Reset New to Current?",
              body: `All ${workspace.changed_count} staged change(s) are replaced by Current. You can Undo this.`,
              confirm: "Reset",
              onConfirm: () => void applyOp({ type: "ResetToBaseline" }),
            })
          }
        >
          Reset…
        </button>
      </div>
      <div className="table-head bank-grid">
        <span>Slot</span>
        <span>A · Current</span>
        <span>B · New</span>
        <span>Category</span>
        <span />
      </div>
      <div className="scroll" ref={scrollRef}>
        <div style={{ height: v.getTotalSize(), position: "relative" }}>
          {v.getVirtualItems().map((vi) => {
            const s = slots[vi.index];
            const sel = bankSel.ids.has(s.slot);
            const focus = bankSel.focus === s.slot;
            const inReplace = replaceRange && s.slot >= replaceRange.from && s.slot <= replaceRange.to;
            const gapBefore = t && t.type === "gap" && t.gap === s.slot;
            const gapAfter = t && t.type === "gap" && t.gap === s.slot + 1 && vi.index === slots.length - 1;
            const same = s.current && s.new && s.current.blob_hash === s.new.blob_hash;
            return (
              <div
                key={s.slot}
                className={[
                  "row bank-grid",
                  sel ? "selected" : "",
                  focus ? "focus" : "",
                  s.changed ? "changed" : "",
                  s.slot % 100 === 0 ? "bank-start" : "",
                  inReplace ? (replaceRange!.overflow ? "drop-bad" : "drop-replace") : "",
                  gapBefore ? "gap-before" : "",
                  gapAfter ? "gap-after" : "",
                ].join(" ")}
                style={{ transform: `translateY(${vi.start}px)`, height: BANK_ROW_H }}
                onMouseDown={(e) => {
                  if (e.button !== 0) return;
                  const mods = { meta: e.metaKey || e.ctrlKey, shift: e.shiftKey };
                  let next = bankSel;
                  if (!sel || mods.meta || mods.shift) {
                    next = clickSelect(bankSel, order, s.slot, mods);
                    set({ bankSel: next, pane: "bank" });
                  } else set({ bankSel: { ...bankSel, focus: s.slot }, pane: "bank" });
                  const frozen = orderedSelection(next, order).sort((a, b) => a - b);
                  beginPointerDrag(e, { source: "bank", bankSlots: frozen }, () => {
                    if (sel && !mods.meta && !mods.shift) set({ bankSel: clickSelect(bankSel, order, s.slot, { meta: false, shift: false }) });
                  });
                }}
                onDoubleClick={() => s.new && void useApp.getState().auditionHash(s.new.blob_hash, `${slot3(s.slot)} B · ${s.new.name}`, true, "B")}
              >
                <span className="mono slotno">{slot3(s.slot)}</span>
                <span className={`ellipsis ${s.changed ? "muted strike" : "muted"}`}>{s.current?.name ?? "—"}</span>
                <span className="ellipsis">{s.new ? s.new.name : <i className="warn">empty</i>}</span>
                <span>{s.new && <CategoryTag category={s.new.category} manual={s.new.manual_category} />}</span>
                <span className="chg" title={s.changed ? "Will be written" : same ? "Same program" : ""}>
                  {s.changed ? "●" : ""}
                </span>
              </div>
            );
          })}
        </div>
      </div>
      {drag.active && (
        <div className="drag-ghost" style={{ left: drag.x + 14, top: drag.y + 10 }}>
          {drag.source === "library" ? `${drag.libIds.length} program(s)` : `${drag.bankSlots.length} slot(s)`}
          {t && t.type === "replace" && ` → replace ${slot3(t.slot)}–${slot3(Math.min(499, t.slot + drag.libIds.length - 1))}`}
          {replaceRange?.overflow && ` · too long: latest start ${slot3(500 - drag.libIds.length)}`}
          {t && t.type === "gap" && ` → insert before ${t.gap === 500 ? "end" : slot3(t.gap)}`}
          {!t && " (drop on the bank)"}
        </div>
      )}
    </section>
  );
}
