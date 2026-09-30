import { useVirtualizer } from "@tanstack/react-virtual";
import { useEffect, useMemo, useRef } from "react";
import { api } from "../api/backend";
import type { OccurrenceRow } from "../api/types";
import { defaultVisibleSources, filterLibrary, formatAddress, type LibSort } from "../lib/library";
import { clickSelect, orderedSelection, pruneToVisible } from "../lib/selection";
import { useApp } from "../stores/app";
import { beginPointerDrag } from "./dragging";
import { CategoryMenu, CategoryTag } from "./common";

export const ROW_H = 24;

export function useFilteredLibrary(): OccurrenceRow[] {
  const occurrences = useApp((s) => s.occurrences);
  const filter = useApp((s) => s.filter);
  const sources = useApp((s) => s.sources);
  const visible = useMemo(() => defaultVisibleSources(sources), [sources]);
  return useMemo(() => filterLibrary(occurrences, filter, visible), [occurrences, filter, visible]);
}

export function LibraryPane({ searchRef }: { searchRef: React.RefObject<HTMLInputElement | null> }) {
  const rows = useFilteredLibrary();
  const { filter, setFilter, libSel, set, pane, applyMeta, workspace, bankSel, applyOp, openDialog, toast, error } = useApp();
  const order = useMemo(() => rows.map((r) => r.id), [rows]);
  const scrollRef = useRef<HTMLDivElement>(null);
  const v = useVirtualizer({ count: rows.length, getScrollElement: () => scrollRef.current, estimateSize: () => ROW_H, overscan: 20 });

  // Filtering removes hidden selections.
  useEffect(() => {
    const pruned = pruneToVisible(libSel, order);
    if (pruned !== libSel) {
      set({ libSel: pruned });
      toast("info", `${pruned.ids.size} selected after filtering.`);
    }
  }, [order]); // eslint-disable-line react-hooks/exhaustive-deps

  // Keep focus visible.
  useEffect(() => {
    if (libSel.focus) {
      const i = order.indexOf(libSel.focus);
      if (i >= 0) v.scrollToIndex(i, { align: "auto" });
    }
  }, [libSel.focus]); // eslint-disable-line react-hooks/exhaustive-deps

  const selected = orderedSelection(libSel, order);
  const selRows = () => {
    const byId = new Map(rows.map((r) => [r.id, r]));
    return selected.map((id) => byId.get(id)!).filter(Boolean);
  };

  const placeAtFocus = async () => {
    const start = bankSel.focus ?? 0;
    if (!selected.length) return;
    await applyOp({ type: "ReplaceFromLibrary", start, occurrence_ids: selected });
  };

  const copy = async () => {
    try {
      const n = await api().copyPrograms(selRows().map((r) => ({ blob_hash: r.exact_hash, occurrence_id: r.id })));
      set({ clipboardCount: n });
      toast("info", `Copied ${n} program(s). Paste into New with ⌘V.`);
    } catch (e) {
      error(e);
    }
  };

  return (
    <section className={`pane library ${pane === "library" ? "active-pane" : ""}`} onMouseDown={() => pane !== "library" && set({ pane: "library" })}>
      <div className="pane-head">
        <h2>Library</h2>
        <input
          ref={searchRef}
          className="search"
          placeholder="Search name, label, category, source, address  (⌘F)"
          value={filter.search}
          onChange={(e) => setFilter({ search: e.target.value })}
          onKeyDown={(e) => {
            if (e.key === "Escape") (e.target as HTMLInputElement).blur();
          }}
        />
        <select value={filter.sort} onChange={(e) => setFilter({ sort: e.target.value as LibSort })} title="Sort">
          <option value="source">Source order</option>
          <option value="name">Name</option>
          <option value="category">Category</option>
          <option value="address">Original address</option>
        </select>
      </div>
      <div className="pane-tools">
        <span className="selcount">{selected.length ? `${selected.length} selected` : `${rows.length} sounds`}</span>
        <button disabled={!selected.length || !workspace} onClick={() => void placeAtFocus()} title="Replace New starting at the focused bank slot">
          Place at {String(bankSel.focus ?? 0).padStart(3, "0")}
        </button>
        <button disabled={!selected.length} onClick={() => void copy()} title="⌘C">
          Copy
        </button>
        <CategoryMenu disabled={!selected.length} onPick={(c) => void applyMeta({ type: "SetCategory", occurrence_ids: selected, category: c })} />
        <button
          disabled={!selected.length}
          onClick={() => {
            const allFav = selRows().every((r) => r.favorite);
            void applyMeta({ type: "SetFavorite", occurrence_ids: selected, favorite: !allFav });
          }}
        >
          ★
        </button>
        <button disabled={!selected.length} onClick={() => openDialog({ kind: "labels", occurrenceIds: selected })}>
          Labels…
        </button>
        <button
          disabled={!selected.length}
          onClick={() => openDialog({ kind: "exportSelection", items: selRows().map((r, i) => ({ name: r.display_name, blob_hash: r.exact_hash, defaultSlot: r.address != null && r.address < 500 ? r.address : i })) })}
        >
          Export…
        </button>
      </div>
      <div className="table-head lib-grid">
        <span />
        <span>Name</span>
        <span>Category</span>
        <span>Source</span>
        <span>Addr</span>
        <span title="Duplicates">Dup</span>
      </div>
      <div className="scroll" ref={scrollRef} tabIndex={-1}>
        {rows.length === 0 && <div className="empty">{useApp.getState().occurrences.length ? "No sounds match the filters." : "Drop .syx files here or use Import .syx."}</div>}
        <div style={{ height: v.getTotalSize(), position: "relative" }}>
          {v.getVirtualItems().map((vi) => {
            const r = rows[vi.index];
            const sel = libSel.ids.has(r.id);
            const focus = libSel.focus === r.id;
            return (
              <div
                key={r.id}
                className={`row lib-grid ${sel ? "selected" : ""} ${focus ? "focus" : ""}`}
                style={{ transform: `translateY(${vi.start}px)`, height: ROW_H }}
                onMouseDown={(e) => {
                  if (e.button !== 0) return;
                  const mods = { meta: e.metaKey || e.ctrlKey, shift: e.shiftKey };
                  let next = libSel;
                  if (!sel || mods.meta || mods.shift) {
                    next = clickSelect(libSel, order, r.id, mods);
                    set({ libSel: next, pane: "library" });
                  } else {
                    set({ libSel: { ...libSel, focus: r.id }, pane: "library" });
                  }
                  const frozen = orderedSelection(next, order);
                  beginPointerDrag(e, { source: "library", libIds: frozen }, () => {
                    // Plain click on an already-selected row: collapse to it.
                    if (sel && !mods.meta && !mods.shift) set({ libSel: clickSelect(libSel, order, r.id, { meta: false, shift: false }) });
                  });
                }}
                onDoubleClick={() => void useApp.getState().auditionHash(r.exact_hash, r.display_name, true)}
                title={`${r.stored_name && r.vault_label ? `Stored name: ${r.stored_name} · ` : ""}format byte ${r.format_version}${r.params_available ? "" : " · parameters unavailable (unsupported layout)"}`}
              >
                <span
                  className={`fav ${r.favorite ? "on" : ""}`}
                  onMouseDown={(e) => e.stopPropagation()}
                  onClick={() => void applyMeta({ type: "SetFavorite", occurrence_ids: [r.id], favorite: !r.favorite })}
                  aria-label="favorite"
                >
                  {r.favorite ? "★" : "☆"}
                </span>
                <span className="name">
                  {r.display_name}
                  {r.vault_label && <span className="lbl" title="Vault label (not written to the synth)">L</span>}
                  {r.badges.map((b) => (
                    <span key={b} className="badge small">
                      {b}
                    </span>
                  ))}
                  {r.noncanonical && <span className="badge warn small" title="Non-canonical packing in the original file; bytes preserved">NC</span>}
                </span>
                <CategoryTag category={r.effective_category} manual={!!r.manual_category} score={r.auto_score} />
                <span className="muted ellipsis">{r.source_name}</span>
                <span className="mono">{formatAddress(r)}</span>
                <span className="muted">{r.dup_exact > 0 ? `=${r.dup_exact}` : r.dup_name_only > 0 ? `≈${r.dup_name_only}` : ""}</span>
              </div>
            );
          })}
        </div>
      </div>
    </section>
  );
}
