import { useEffect, useRef } from "react";
import { api } from "../api/backend";
import { CATEGORIES } from "../api/types";
import { BankPane, useVisibleSlots } from "../components/BankPane";
import { BottomBar } from "../components/BottomBar";
import { LibraryPane, useFilteredLibrary } from "../components/LibraryPane";
import { Sidebar } from "../components/Sidebar";
import { TopBar } from "../components/TopBar";
import { ConfirmDialog, DiagnosticsDialog, ExportSelectionDialog, HistoryDialog, LabelsDialog, MoveToDialog, SwapDialog } from "../components/dialogs/EditDialogs";
import { ConnectDialog } from "../components/dialogs/ConnectDialog";
import { ImportDialog, ImportResultsDialog } from "../components/dialogs/ImportDialog";
import {
  CloseBlockedDialog,
  ProtectDialog,
  ReconcileDialog,
  RecoveryDialog,
  ReviewDialog,
  SyncPartialDialog,
  WriteResultDialog,
} from "../components/dialogs/SafetyDialogs";
import { useBackendEvents } from "../hooks/useEvents";
import { slot3 } from "../lib/format";
import { isTypingTarget, moveFocus, orderedSelection, selectAll } from "../lib/selection";
import { useApp } from "../stores/app";

function Dialogs() {
  const d = useApp((s) => s.dialog);
  if (!d) return null;
  switch (d.kind) {
    case "connect":
      return <ConnectDialog />;
    case "import":
      return <ImportDialog previews={d.previews} />;
    case "importResults":
      return <ImportResultsDialog results={d.results} previews={d.previews} />;
    case "protect":
      return <ProtectDialog pending={d.pending} />;
    case "reconcile":
      return <ReconcileDialog workspaceId={d.workspaceId} snapshotId={d.snapshotId} slots={d.slots} purpose={d.purpose} sessionId={d.sessionId} />;
    case "review":
      return <ReviewDialog review={d.review} />;
    case "writeResult":
      return <WriteResultDialog outcome={d.outcome} />;
    case "recovery":
      return <RecoveryDialog session={d.session} report={d.report} />;
    case "moveTo":
      return <MoveToDialog slots={d.slots} />;
    case "copyTo":
      return <MoveToDialog slots={d.slots} copy />;
    case "swap":
      return <SwapDialog slots={d.slots} />;
    case "labels":
      return <LabelsDialog occurrenceIds={d.occurrenceIds} />;
    case "exportSelection":
      return <ExportSelectionDialog items={d.items} />;
    case "history":
      return <HistoryDialog />;
    case "diagnostics":
      return <DiagnosticsDialog />;
    case "confirm":
      return <ConfirmDialog title={d.title} body={d.body} confirm={d.confirm} onConfirm={d.onConfirm} />;
    case "syncPartial":
      return <SyncPartialDialog read={d.read} />;
    case "closeBlocked":
      return <CloseBlockedDialog opKind={d.opKind} opId={d.opId} />;
  }
}

function Toasts() {
  const { toasts, dismissToast } = useApp();
  return (
    <div className="toasts" aria-live="polite">
      {toasts.map((t) => (
        <div key={t.id} className={`toast ${t.kind}`} onClick={() => dismissToast(t.id)}>
          <div>{t.text}</div>
          {t.action && <div className="small muted">{t.action}</div>}
        </div>
      ))}
    </div>
  );
}

/** Auto Audition: single focus changes schedule the focused sound after ~120 ms. */
function useAutoAudition() {
  const lib = useFilteredLibrary();
  const timer = useRef<number | null>(null);
  const libFocus = useApp((s) => s.libSel.focus);
  const bankFocus = useApp((s) => s.bankSel.focus);
  const pane = useApp((s) => s.pane);
  const auto = useApp((s) => s.autoAudition);
  useEffect(() => {
    if (!auto) return;
    const st = useApp.getState();
    const connected = st.status?.state === "Connected" || st.status?.state === "Simulator";
    if (!connected || st.op || st.dialog) return;
    if (timer.current) clearTimeout(timer.current);
    timer.current = window.setTimeout(() => {
      const s = useApp.getState();
      if (s.pane === "library" && s.libSel.focus) {
        const r = lib.find((x) => x.id === s.libSel.focus);
        if (r) void s.auditionHash(r.exact_hash, r.display_name, false);
      } else if (s.pane === "bank" && s.bankSel.focus != null && s.workspace) {
        const c = s.workspace.slots[s.bankSel.focus]?.new;
        if (c) void s.auditionHash(c.blob_hash, `${slot3(s.bankSel.focus)} B · ${c.name}`, false, "B");
      }
    }, 120);
    return () => {
      if (timer.current) clearTimeout(timer.current);
    };
  }, [libFocus, bankFocus, pane, auto]); // eslint-disable-line react-hooks/exhaustive-deps
}

function useKeyboard(searchRef: React.RefObject<HTMLInputElement | null>) {
  const lib = useFilteredLibrary();
  const slots = useVisibleSlots();
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const s = useApp.getState();
      if (e.isComposing) return;
      const meta = e.metaKey || e.ctrlKey;
      const typing = isTypingTarget(e.target);
      if (meta && e.key.toLowerCase() === "f") {
        e.preventDefault();
        searchRef.current?.focus();
        searchRef.current?.select();
        return;
      }
      if (s.dialog || typing) return; // native text editing keeps its own shortcuts
      const libOrder = lib.map((r) => r.id);
      const bankOrder = slots.map((x) => x.slot);
      if (meta && e.key.toLowerCase() === "z") {
        e.preventDefault();
        void (e.shiftKey ? s.redo() : s.undo());
        return;
      }
      if (meta && e.key.toLowerCase() === "a") {
        e.preventDefault();
        if (s.pane === "library") s.set({ libSel: selectAll(libOrder, s.libSel.focus) });
        else s.set({ bankSel: selectAll(bankOrder, s.bankSel.focus) });
        return;
      }
      if (meta && e.key.toLowerCase() === "c") {
        e.preventDefault();
        let items: { blob_hash: string; occurrence_id: string | null }[] = [];
        if (s.pane === "library") {
          const byId = new Map(lib.map((r) => [r.id, r]));
          items = orderedSelection(s.libSel, libOrder).map((id) => ({ blob_hash: byId.get(id)!.exact_hash, occurrence_id: id }));
        } else if (s.workspace) {
          items = orderedSelection(s.bankSel, bankOrder)
            .sort((a, b) => a - b)
            .map((n) => s.workspace!.slots[n].new)
            .filter((c) => !!c)
            .map((c) => ({ blob_hash: c!.blob_hash, occurrence_id: c!.occurrence_id }));
        }
        if (!items.length) return;
        void api()
          .copyPrograms(items)
          .then((n) => {
            s.set({ clipboardCount: n });
            s.toast("info", `Copied ${n} program(s).`);
          })
          .catch(s.error);
        return;
      }
      if (meta && e.key.toLowerCase() === "v") {
        e.preventDefault();
        if (s.bankSel.focus == null) return s.toast("info", "Click a bank slot to paste into.");
        void s.applyOp({ type: "Paste", start: s.bankSel.focus, clipboard: "" });
        return;
      }
      if (meta) return;
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        const d = e.key === "ArrowDown" ? 1 : -1;
        if (s.pane === "library") s.set({ libSel: moveFocus(s.libSel, libOrder, d, e.shiftKey) });
        else s.set({ bankSel: moveFocus(s.bankSel, bankOrder, d, e.shiftKey) });
        return;
      }
      if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
        // Switch panes (Library on the left, Bank on the right).
        e.preventDefault();
        const pane = e.key === "ArrowLeft" ? "library" : "bank";
        s.set({ pane, compactTab: pane });
        return;
      }
      if (e.key === "Enter") {
        e.preventDefault();
        if (s.pane === "library") {
          const r = lib.find((x) => x.id === s.libSel.focus);
          if (r) void s.auditionHash(r.exact_hash, r.display_name, true);
        } else if (s.workspace && s.bankSel.focus != null) {
          const c = s.workspace.slots[s.bankSel.focus].new;
          if (c) void s.auditionHash(c.blob_hash, `${slot3(s.bankSel.focus)} B · ${c.name}`, true, "B");
        }
        return;
      }
      if ((e.key === "a" || e.key === "b") && s.workspace && s.bankSel.focus != null) {
        const slot = s.workspace.slots[s.bankSel.focus];
        const c = e.key === "a" ? slot.current : slot.new;
        if (c) void s.auditionHash(c.blob_hash, `${slot3(slot.slot)} ${e.key.toUpperCase()} · ${c.name}`, true, e.key === "a" ? "A" : "B");
        return;
      }
      if (/^[1-8]$/.test(e.key)) {
        const cat = CATEGORIES[Number(e.key) - 1];
        let ids: string[] = [];
        if (s.pane === "library") ids = orderedSelection(s.libSel, libOrder);
        else if (s.workspace)
          ids = orderedSelection(s.bankSel, bankOrder)
            .map((n) => s.workspace!.slots[n].new?.occurrence_id)
            .filter((x): x is string => !!x);
        if (ids.length) void s.applyMeta({ type: "SetCategory", occurrence_ids: ids, category: cat });
        return;
      }
      if (e.key === "Escape") {
        s.set({ libSel: { ids: new Set(), anchor: null, focus: s.libSel.focus } });
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [lib, slots, searchRef]);
}

export function App() {
  const searchRef = useRef<HTMLInputElement>(null);
  const { init, compact, compactTab, set, op } = useApp();
  useBackendEvents();
  useKeyboard(searchRef);
  useAutoAudition();

  useEffect(() => {
    void init().catch((e) => useApp.getState().error(e));
    const onResize = () => set({ compact: window.innerWidth < 1100 });
    onResize();
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  return (
    <div className={`app ${compact ? "compact" : ""} ${op ? "busy" : ""}`}>
      <TopBar />
      <div className="main">
        {!compact && <Sidebar />}
        {compact && (
          <div className="tabs">
            <button className={compactTab === "library" ? "on" : ""} onClick={() => set({ compactTab: "library", pane: "library" })}>
              Library
            </button>
            <button className={compactTab === "bank" ? "on" : ""} onClick={() => set({ compactTab: "bank", pane: "bank" })}>
              Bank
            </button>
          </div>
        )}
        {(!compact || compactTab === "library") && <LibraryPane searchRef={searchRef} />}
        {(!compact || compactTab === "bank") && <BankPane />}
      </div>
      <BottomBar />
      <Dialogs />
      <Toasts />
    </div>
  );
}
