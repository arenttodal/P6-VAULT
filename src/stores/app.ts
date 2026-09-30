import { create } from "zustand";
import { api } from "../api/backend";
import type * as T from "../api/types";
import { defaultFilter, type LibraryFilter } from "../lib/library";
import { emptySelection, type Selection } from "../lib/selection";
import { markStarted } from "../hooks/useEvents";

export type Pane = "library" | "bank";

export type Dialog =
  | { kind: "connect" }
  | { kind: "import"; previews: T.ImportPreview[] }
  | { kind: "importResults"; results: T.CommitResult[]; previews: T.ImportPreview[] }
  | { kind: "protect"; pending: { hash: string; label: string } | null }
  | { kind: "reconcile"; workspaceId: string; snapshotId: string; slots: T.ReconcileSlot[]; purpose: "sync" | "drift" | "recovery"; sessionId?: string }
  | { kind: "review"; review: T.Review }
  | { kind: "writeResult"; outcome: T.WriteOutcome }
  | { kind: "recovery"; session: T.WriteSessionRow; report: T.InspectReport | null }
  | { kind: "moveTo"; slots: number[] }
  | { kind: "copyTo"; slots: number[] }
  | { kind: "swap"; slots: number[] }
  | { kind: "labels"; occurrenceIds: string[] }
  | { kind: "exportSelection"; items: { name: string; blob_hash: string; defaultSlot: number }[] }
  | { kind: "history" }
  | { kind: "diagnostics" }
  | { kind: "confirm"; title: string; body: string; confirm: string; onConfirm: () => void }
  | { kind: "syncPartial"; read: T.ReadSessionState }
  | { kind: "closeBlocked"; opKind: string; opId: string };

export interface Toast {
  id: number;
  kind: "info" | "error" | "success";
  text: string;
  action?: string | null;
}

export interface RunningOp {
  opId: string;
  kind: string;
  progress: T.Progress | null;
  startedMs: number;
  stopping: boolean;
}

interface State {
  info: T.AppInfo | null;
  status: T.ConnectionStatus | null;
  sources: T.SourceRow[];
  occurrences: T.OccurrenceRow[];
  workspace: T.WorkspaceView | null;
  workspaces: T.WorkspaceRow[];
  filter: LibraryFilter;
  libSel: Selection<string>;
  bankSel: Selection<number>;
  pane: Pane;
  changedOnly: boolean;
  autoAudition: boolean;
  midiChannel: number;
  requested: { label: string; hash: string; which?: "A" | "B" } | null;
  lastSent: { label: string; status: string } | null;
  op: RunningOp | null;
  dialog: Dialog | null;
  toasts: Toast[];
  clipboardCount: number;
  compact: boolean;
  compactTab: Pane | "sidebar";
}

interface Actions {
  set: (p: Partial<State>) => void;
  toast: (kind: Toast["kind"], text: string, action?: string | null) => void;
  dismissToast: (id: number) => void;
  error: (e: unknown) => void;
  init: () => Promise<void>;
  refreshLibrary: () => Promise<void>;
  refreshWorkspace: () => Promise<void>;
  refreshStatus: () => Promise<void>;
  applyOp: (op: T.WorkspaceOp, opts?: { preview?: boolean }) => Promise<boolean>;
  applyMeta: (op: T.MetaOp) => Promise<boolean>;
  undo: () => Promise<void>;
  redo: () => Promise<void>;
  setFilter: (p: Partial<LibraryFilter>) => void;
  auditionHash: (hash: string, label: string, force: boolean, which?: "A" | "B") => Promise<void>;
  startOp: (kind: string, start: () => Promise<string>) => Promise<void>;
  openDialog: (d: Dialog) => void;
  closeDialog: () => void;
}

let toastSeq = 1;

export const useApp = create<State & Actions>((set, get) => ({
  info: null,
  status: null,
  sources: [],
  occurrences: [],
  workspace: null,
  workspaces: [],
  filter: defaultFilter,
  libSel: emptySelection(),
  bankSel: emptySelection(),
  pane: "bank",
  changedOnly: false,
  autoAudition: false,
  midiChannel: 1,
  requested: null,
  lastSent: null,
  op: null,
  dialog: null,
  toasts: [],
  clipboardCount: 0,
  compact: false,
  compactTab: "bank",

  set: (p) => set(p),
  toast: (kind, text, action) => {
    const id = toastSeq++;
    set((s) => ({ toasts: [...s.toasts.slice(-4), { id, kind, text, action }] }));
    if (kind !== "error") setTimeout(() => get().dismissToast(id), 5000);
  },
  dismissToast: (id) => set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })),
  error: (e) => {
    const err = e as T.ApiError;
    if (err && typeof err === "object" && "message" in err) get().toast("error", err.message, err.action);
    else get().toast("error", String(e));
  },

  init: async () => {
    const b = api();
    const [info, status] = await Promise.all([b.appInfo(), b.connectionStatus()]);
    let auto = false;
    let ch = 1;
    try {
      auto = (await b.getSetting("autoAudition")) === "true";
      ch = Number((await b.getSetting("midiChannel")) ?? "1") || 1;
    } catch {
      /* defaults */
    }
    set({ info, status, autoAudition: auto, midiChannel: ch });
    try {
      const ctx = JSON.parse((await b.getSetting("uiContext")) ?? "null");
      if (ctx && typeof ctx === "object") {
        set({
          filter: { ...defaultFilter, ...(ctx.filter ?? {}) },
          changedOnly: !!ctx.changedOnly,
          pane: ctx.pane === "library" ? "library" : "bank",
          bankSel: typeof ctx.bankFocus === "number" ? { ids: new Set([ctx.bankFocus]), anchor: ctx.bankFocus, focus: ctx.bankFocus } : emptySelection(),
        });
      }
    } catch {
      /* ignore stale context */
    }
    await Promise.all([get().refreshLibrary(), get().refreshWorkspace()]);
    const s = info.unfinished_sessions[0];
    if (s) set({ dialog: { kind: "recovery", session: s, report: null } });
  },

  refreshLibrary: async () => {
    try {
      const [sources, occurrences] = await Promise.all([api().listSources(), api().listOccurrences()]);
      set({ sources, occurrences });
    } catch (e) {
      get().error(e);
    }
  },

  refreshWorkspace: async () => {
    try {
      const info = await api().appInfo();
      const workspaces = await api().listWorkspaces();
      const ws = info.active_workspace ? await api().workspaceView(info.active_workspace) : null;
      set({ info, workspace: ws, workspaces });
    } catch (e) {
      get().error(e);
    }
  },

  refreshStatus: async () => {
    try {
      set({ status: await api().connectionStatus() });
    } catch (e) {
      get().error(e);
    }
  },

  applyOp: async (op) => {
    const ws = get().workspace;
    if (!ws) {
      get().toast("error", "No bank yet. Sync from the synth, or build one from an imported bank.");
      return false;
    }
    try {
      const rev = await api().applyOp(ws.id, ws.revision, op);
      if (rev === ws.revision) get().toast("info", "No change.");
      await get().refreshWorkspace();
      return true;
    } catch (e) {
      get().error(e);
      if ((e as T.ApiError)?.code === "RevisionConflict") await get().refreshWorkspace();
      return false;
    }
  },

  applyMeta: async (op) => {
    const ws = get().workspace;
    if (!ws) {
      get().toast("error", "Create or sync a bank first; metadata edits are recorded in its undo history.");
      return false;
    }
    try {
      await api().applyMeta(ws.id, ws.revision, op);
      await Promise.all([get().refreshWorkspace(), get().refreshLibrary()]);
      return true;
    } catch (e) {
      get().error(e);
      if ((e as T.ApiError)?.code === "RevisionConflict") await get().refreshWorkspace();
      return false;
    }
  },

  undo: async () => {
    const ws = get().workspace;
    if (!ws?.can_undo) return;
    try {
      await api().undo(ws.id, ws.revision);
      get().toast("info", `Undid: ${ws.undo_label}`);
      await Promise.all([get().refreshWorkspace(), get().refreshLibrary()]);
    } catch (e) {
      get().error(e);
    }
  },
  redo: async () => {
    const ws = get().workspace;
    if (!ws?.can_redo) return;
    try {
      await api().redo(ws.id, ws.revision);
      get().toast("info", `Redid: ${ws.redo_label}`);
      await Promise.all([get().refreshWorkspace(), get().refreshLibrary()]);
    } catch (e) {
      get().error(e);
    }
  },

  setFilter: (p) => set((s) => ({ filter: { ...s.filter, ...p } })),

  auditionHash: async (hash, label, force, which) => {
    set({ requested: { hash, label, which } });
    try {
      const ack = await api().audition(hash, label, force);
      if (ack.status === "needs_protection") {
        set({ dialog: { kind: "protect", pending: { hash, label } } });
      } else if (ack.status !== "queued") {
        get().toast("info", ack.message ?? ack.status);
        set({ requested: null });
      }
    } catch (e) {
      get().error(e);
    }
  },

  startOp: async (kind, start) => {
    if (get().op) {
      get().toast("error", "Another operation is running.");
      return;
    }
    try {
      const opId = await start();
      if (!markStarted(opId)) set({ op: { opId, kind, progress: null, startedMs: Date.now(), stopping: false } });
    } catch (e) {
      get().error(e);
    }
  },

  openDialog: (d) => set({ dialog: d }),
  closeDialog: () => set({ dialog: null }),
}));

/** Persist lightweight UI context (debounced; never sound data). */
let ctxTimer: ReturnType<typeof setTimeout> | null = null;
let lastCtx = "";
useApp.subscribe((s) => {
  if (!s.info) return;
  const ctx = JSON.stringify({ filter: s.filter, changedOnly: s.changedOnly, pane: s.pane, bankFocus: s.bankSel.focus });
  if (ctx === lastCtx) return;
  lastCtx = ctx;
  if (ctxTimer) clearTimeout(ctxTimer);
  ctxTimer = setTimeout(() => {
    try {
      void api().setSetting("uiContext", ctx).catch(() => {});
    } catch {
      /* backend not ready */
    }
  }, 600);
});
