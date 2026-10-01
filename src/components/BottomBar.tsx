import { api } from "../api/backend";
import { fmtDuration, slot3 } from "../lib/format";
import { useApp } from "../stores/app";
import { useFilteredLibrary } from "./LibraryPane";

export function BottomBar() {
  const s = useApp();
  const { status, workspace, bankSel, libSel, pane, autoAudition, requested, lastSent, op, midiChannel, set, auditionHash, error, toast } = s;
  const lib = useFilteredLibrary();
  const connected = status?.state === "Connected" || status?.state === "Simulator";
  const blocked = !!op;

  const focusSlot = bankSel.focus != null && workspace ? workspace.slots[bankSel.focus] : null;
  const focusLib = libSel.focus ? lib.find((r) => r.id === libSel.focus) : null;

  const a = () => focusSlot?.current && void auditionHash(focusSlot.current.blob_hash, `${slot3(focusSlot.slot)} A · ${focusSlot.current.name}`, true, "A");
  const b = () => focusSlot?.new && void auditionHash(focusSlot.new.blob_hash, `${slot3(focusSlot.slot)} B · ${focusSlot.new.name}`, true, "B");
  const same = focusSlot?.current && focusSlot?.new && focusSlot.current.blob_hash === focusSlot.new.blob_hash;

  const toggleAuto = async () => {
    const v = !autoAudition;
    set({ autoAudition: v });
    try {
      await api().setSetting("autoAudition", String(v));
    } catch {
      /* preference only */
    }
  };

  const pct = op?.progress && op.progress.total ? Math.round((op.progress.done / op.progress.total) * 100) : 0;
  const elapsed = op ? Date.now() - op.startedMs : 0;

  return (
    <footer className="bottombar">
      <div className="focus-info">
        {pane === "bank" && focusSlot ? (
          <>
            <span className="mono">{slot3(focusSlot.slot)}</span>
            <span className="muted">A</span> {focusSlot.current?.name ?? "—"}
            <span className="muted">B</span> {focusSlot.new?.name ?? <i>empty</i>}
            {same && <span className="badge small">Same program</span>}
          </>
        ) : pane === "library" && focusLib ? (
          <>
            <b>{focusLib.display_name}</b> <span className="muted">{focusLib.source_name}</span>
          </>
        ) : (
          <span className="muted">Nothing focused</span>
        )}
      </div>
      <div className="audition">
        <button aria-pressed={requested?.which === "A"} disabled={!connected || blocked || !focusSlot?.current} onClick={a} title="Load Current (A) into the edit buffer (A)">
          A · Current
        </button>
        <button aria-pressed={requested?.which === "B"} disabled={!connected || blocked || !focusSlot?.new || !!same} onClick={b} title={focusSlot && !focusSlot.new ? "Slot is empty" : "Load New (B) into the edit buffer (B)"}>
          B · New
        </button>
        <button
          disabled={!connected || blocked || (pane === "library" ? !focusLib : !focusSlot?.new)}
          onClick={() => {
            if (pane === "library" && focusLib) void auditionHash(focusLib.exact_hash, focusLib.display_name, true);
            else b();
          }}
          title="Enter"
        >
          Audition
        </button>
        <label className="toggle" title="Load the focused sound after you move focus (120 ms debounce)">
          <input type="checkbox" checked={autoAudition} onChange={() => void toggleAuto()} /> Auto
        </label>
        <span className="sep" />
        <button
          disabled={!connected || blocked}
          onClick={() =>
            void api()
              .testNote(midiChannel)
              .catch((e) => error(e))
          }
          title="Send middle C (note 60, velocity 80) for 600 ms"
        >
          Test note
        </button>
        <select value={midiChannel} onChange={(e) => { const c = Number(e.target.value); set({ midiChannel: c }); void api().setSetting("midiChannel", String(c)).catch(() => {}); }} title="MIDI channel for test note / panic">
          {Array.from({ length: 16 }, (_, i) => (
            <option key={i} value={i + 1}>
              Ch {i + 1}
            </option>
          ))}
        </select>
        <button
          className="panic"
          disabled={!connected}
          onClick={() =>
            void api()
              .panic(midiChannel)
              .then(() => toast("info", "Notes released."))
              .catch((e) => error(e))
          }
        >
          Panic
        </button>
      </div>
      <div className="audition-state" aria-live="polite">
        {requested && <span>Requested: {requested.label}</span>}
        {lastSent && <span className="muted"> · Sent: {lastSent.label}</span>}
      </div>
      {op && (
        <div className="progress">
          <span>
            {op.kind === "sync" ? "Reading bank" : op.kind === "prepare" ? "Backing up before review" : op.kind === "write" ? "Writing" : op.kind === "inspect" ? "Inspecting" : op.kind}
            {op.progress?.phase && op.progress.phase !== op.kind ? ` (${op.progress.phase})` : ""}
            {op.progress ? ` ${op.progress.done}/${op.progress.total}` : ""}
            {op.progress?.slot != null ? ` · slot ${slot3(op.progress.slot)}` : ""} · {fmtDuration(elapsed)}
          </span>
          <div className="bar">
            <div style={{ width: `${pct}%` }} />
          </div>
          <button
            disabled={op.stopping}
            onClick={() => {
              set({ op: { ...op, stopping: true } });
              void api().stopOperation(op.opId);
            }}
          >
            {op.kind === "write" ? "Stop after current program" : "Stop"}
          </button>
        </div>
      )}
    </footer>
  );
}
