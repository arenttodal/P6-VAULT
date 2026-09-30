import { useEffect } from "react";
import { api } from "../api/backend";
import type * as T from "../api/types";
import { useApp } from "../stores/app";

const finishedEarly = new Set<string>();

export function markStarted(opId: string): boolean {
  // True if the op already finished before its id was known to the UI.
  return finishedEarly.delete(opId);
}

async function handleDone(ev: T.OpDone) {
  const s = useApp.getState();
  if (!s.op || s.op.opId !== ev.op_id) finishedEarly.add(ev.op_id);
  useApp.setState({ op: null });
  await Promise.all([s.refreshWorkspace(), s.refreshLibrary(), s.refreshStatus()]);
  if (!ev.ok) {
    s.error(ev.error);
    return;
  }
  const st = useApp.getState();
  switch (ev.kind) {
    case "sync": {
      const r = ev.result as T.SyncResult;
      if (!r.read.snapshot_id) {
        st.openDialog({ kind: "syncPartial", read: r.read });
      } else if (r.created_workspace) {
        st.toast("success", "Current captured from the synth (500 programs). New is ready to organize.");
      } else if (r.rebased) {
        st.toast("success", `Current refreshed from the synth. Staged changes kept (${r.reconcile.filter((x) => x.resolution === "KeepStaged").length} slot(s)).`);
      } else if (r.workspace_id) {
        st.openDialog({ kind: "reconcile", workspaceId: r.workspace_id, snapshotId: r.read.snapshot_id, slots: r.reconcile, purpose: "sync" });
      }
      break;
    }
    case "prepare": {
      const o = ev.result as T.PrepareOutcome;
      if (o.kind === "Ready") st.openDialog({ kind: "review", review: o });
      else if (o.kind === "NoChanges") st.toast("info", "The synth already matches New. Nothing to write. (A fresh backup was saved.)");
      else if (o.kind === "Drift" && st.workspace)
        st.openDialog({ kind: "reconcile", workspaceId: st.workspace.id, snapshotId: o.live_snapshot_id, slots: o.slots, purpose: "drift" });
      break;
    }
    case "write": {
      st.openDialog({ kind: "writeResult", outcome: ev.result as T.WriteOutcome });
      break;
    }
    case "inspect": {
      const rep = ev.result as T.InspectReport;
      const d = st.dialog;
      if (d?.kind === "recovery") st.openDialog({ ...d, report: rep });
      break;
    }
  }
}

export function useBackendEvents() {
  useEffect(() => {
    const unsubs: Promise<() => void>[] = [];
    const b = api();
    unsubs.push(
      b.on<T.Progress>("op-progress", (p) => {
        const op = useApp.getState().op;
        if (op && op.opId === p.op_id) useApp.setState({ op: { ...op, progress: p } });
      }),
    );
    unsubs.push(b.on<T.OpDone>("op-done", (d) => void handleDone(d)));
    unsubs.push(b.on<T.ConnectionStatus>("connection", (s) => useApp.setState({ status: s })));
    unsubs.push(
      b.on<{ id: number; label: string; status: string; error?: string }>("audition", (a) => {
        useApp.setState({ lastSent: { label: a.label, status: a.status } });
        if (a.status === "failed") useApp.getState().toast("error", `Audition failed: ${a.error}`);
      }),
    );
    unsubs.push(
      b.on<{ op_id: string; kind: string }>("close-blocked", (e) => useApp.getState().openDialog({ kind: "closeBlocked", opKind: e.kind, opId: e.op_id })),
    );
    unsubs.push(
      b.onFileDrop((paths) => {
        const syx = paths.filter((p) => p.length > 0);
        if (syx.length) void startImport(syx);
      }),
    );
    // Hardware port liveness.
    const t = setInterval(() => {
      const st = useApp.getState().status;
      if (st && st.state === "Connected" && !st.busy) void b.checkPorts().then((s) => useApp.setState({ status: s }));
    }, 3000);
    return () => {
      clearInterval(t);
      unsubs.forEach((u) => void u.then((f) => f()));
    };
  }, []);
}

export async function startImport(paths: string[]) {
  const s = useApp.getState();
  try {
    const previews = await api().previewImports(paths);
    s.openDialog({ kind: "import", previews });
  } catch (e) {
    s.error(e);
  }
}
