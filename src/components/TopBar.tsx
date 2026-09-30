import { api } from "../api/backend";
import { startImport } from "../hooks/useEvents";
import { useApp } from "../stores/app";

export function TopBar() {
  const { status, workspace, op, startOp, openDialog, toast, error, undo, redo } = useApp();
  const connected = status?.state === "Connected" || status?.state === "Simulator";
  const busy = !!op;
  const changed = workspace?.changed_count ?? 0;

  const doImport = async () => {
    const files = await api().pickFiles();
    if (files?.length) await startImport(files);
  };

  const doExport = async () => {
    if (!workspace) return toast("error", "No bank to export yet.");
    if (workspace.empty_count > 0) return toast("error", `New has ${workspace.empty_count} empty slot(s); a full-bank export needs all 500.`);
    const path = await api().saveFile(`P6-New-bank.syx`);
    if (!path) return;
    try {
      const n = await api().exportBank(workspace.id, path);
      toast("success", `Exported 500 programs (${n.toLocaleString()} bytes) and verified the file. No MIDI was sent.`);
    } catch (e) {
      error(e);
    }
  };

  const doReview = () => {
    if (!workspace) return;
    if (!connected) return toast("error", "Connect the synth to review and write.");
    if (!workspace.writable_baseline) return toast("error", "Sync Current from the synth first so the app knows exactly what is on it.");
    if (workspace.empty_count > 0) return toast("error", `Fill the ${workspace.empty_count} empty slot(s) first.`);
    void startOp("prepare", () => api().prepareReview(workspace.id));
  };

  const stateLabel = !status ? "…" : status.state === "Simulator" ? "Simulator connected" : status.state === "Connected" ? `Connected · ${status.kind}` : status.state;

  return (
    <header className="topbar">
      <div className="brand">P6 Vault</div>
      {status?.simulator_mode && <span className="badge sim" title="Simulator mode uses a separate library. Nothing here touches real hardware.">SIMULATOR</span>}
      <button className={`conn ${connected ? "ok" : "off"}`} onClick={() => openDialog({ kind: "connect" })} title={status?.description ?? "Connect"}>
        <span className="dot" aria-hidden /> {stateLabel}
      </button>
      <div className="spacer" />
      <button onClick={() => void undo()} disabled={!workspace?.can_undo} title={workspace?.undo_label ? `Undo: ${workspace.undo_label} (⌘Z)` : "Undo (⌘Z)"}>
        Undo
      </button>
      <button onClick={() => void redo()} disabled={!workspace?.can_redo} title={workspace?.redo_label ? `Redo: ${workspace.redo_label} (⇧⌘Z)` : "Redo (⇧⌘Z)"}>
        Redo
      </button>
      <span className="sep" />
      <button onClick={() => void doImport()}>Import .syx</button>
      <button disabled={!connected || busy} onClick={() => void startOp("sync", () => api().syncStart(null))} title="Read user programs 000–499 from the synth">
        Sync from P6
      </button>
      <button onClick={() => void doExport()} disabled={!workspace}>
        Export New
      </button>
      <button className="primary" disabled={!workspace || busy || changed === 0} onClick={doReview} title="Back up the synth, then review the exact write list">
        Review {changed} change{changed === 1 ? "" : "s"}
      </button>
    </header>
  );
}
