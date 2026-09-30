import { useEffect, useState } from "react";
import { api } from "../../api/backend";
import type { DiagEntry, OpPreview, ProtectedBuffer, SnapshotRow, WriteSessionRow, WorkspaceOp } from "../../api/types";
import { fmtTime, slot3 } from "../../lib/format";
import { useApp } from "../../stores/app";
import { Modal } from "../Modal";

function SlotInput({ value, onChange, label }: { value: number; onChange: (n: number) => void; label: string }) {
  const [text, setText] = useState(String(value).padStart(3, "0"));
  const n = /^\d{1,3}$/.test(text.trim()) ? Number(text.trim()) : NaN;
  const bad = !(n >= 0 && n <= 499);
  return (
    <label>
      {label}
      <input
        className="mono"
        inputMode="numeric"
        value={text}
        onChange={(e) => {
          setText(e.target.value);
          const v = e.target.value.trim();
          if (/^\d{1,3}$/.test(v) && Number(v) <= 499) onChange(Number(v));
        }}
        onFocus={(e) => e.target.select()}
        aria-invalid={bad}
      />
      {bad && <span className="error small">Enter a slot from 000 to 499.</span>}
    </label>
  );
}

function OpPreviewList({ preview }: { preview: OpPreview | null }) {
  if (!preview) return null;
  return (
    <div className="scroll-box small">
      <p>
        {preview.description}: {preview.changes.length} slot(s) change.
      </p>
      <table className="simple">
        <tbody>
          {preview.changes.slice(0, 300).map((c) => (
            <tr key={c.slot}>
              <td className="mono">{slot3(c.slot)}</td>
              <td className="muted">{c.before ?? "empty"}</td>
              <td>→ {c.after ?? "empty"}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function usePreview(op: WorkspaceOp | null) {
  const ws = useApp((s) => s.workspace);
  const [preview, setPreview] = useState<OpPreview | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const key = JSON.stringify(op);
  useEffect(() => {
    if (!ws || !op) return;
    let live = true;
    api()
      .previewOp(ws.id, op)
      .then((p) => live && (setPreview(p), setErr(null)))
      .catch((e) => live && (setPreview(null), setErr(e.message ?? String(e))));
    return () => {
      live = false;
    };
  }, [key, ws?.revision]); // eslint-disable-line react-hooks/exhaustive-deps
  return { preview, err };
}

export function MoveToDialog({ slots, copy }: { slots: number[]; copy?: boolean }) {
  const { closeDialog, applyOp } = useApp();
  const [target, setTarget] = useState(slots[0] ?? 0);
  const op: WorkspaceOp = copy ? { type: "CopySlots", slots, start: target } : { type: "MoveToSlot", slots, target };
  const { preview, err } = usePreview(op);
  return (
    <Modal
      title={copy ? `Copy ${slots.length} slot(s)` : `Move ${slots.length} slot(s)`}
      onClose={closeDialog}
      wide
      footer={
        <>
          <button onClick={closeDialog}>Cancel</button>
          <button className="primary" disabled={!!err} onClick={() => void applyOp(op).then((ok) => ok && closeDialog())}>
            {copy ? "Copy" : "Move"}
          </button>
        </>
      }
    >
      <p className="muted small">
        {copy
          ? "Copies replace the destination slots (nothing shifts). The originals stay where they are."
          : "The selected block starts at the chosen slot after the move; the other programs keep their order and close up around it. Nothing is lost or duplicated."}{" "}
        Highest valid start: {slot3(500 - slots.length)}.
      </p>
      <SlotInput label={copy ? "First destination slot" : "Final first slot"} value={target} onChange={setTarget} />
      {err && <p className="error">{err}</p>}
      <OpPreviewList preview={preview} />
    </Modal>
  );
}

export function SwapDialog({ slots }: { slots: number[] }) {
  const { closeDialog, applyOp } = useApp();
  const sorted = [...slots].sort((a, b) => a - b);
  const contiguous = sorted.every((s, i) => i === 0 || s === sorted[i - 1] + 1);
  const [a] = useState(sorted[0] ?? 0);
  const len = sorted.length;
  const [b, setB] = useState(Math.min(499, (sorted[0] ?? 0) + 100));
  const op: WorkspaceOp = { type: "SwapRanges", a, b, len };
  const { preview, err } = usePreview(contiguous ? op : null);
  return (
    <Modal
      title="Swap with range"
      onClose={closeDialog}
      wide
      footer={
        <>
          <button onClick={closeDialog}>Cancel</button>
          <button className="primary" disabled={!contiguous || !!err} onClick={() => void applyOp(op).then((ok) => ok && closeDialog())}>
            Swap
          </button>
        </>
      }
    >
      {!contiguous ? (
        <p className="error">Select one contiguous range of slots to swap.</p>
      ) : (
        <>
          <p>
            Swap {slot3(a)}–{slot3(a + len - 1)} ({len} slots) with the same number of slots starting at:
          </p>
          <SlotInput label="Other range start" value={b} onChange={setB} />
          {err && <p className="error">{err}</p>}
          <OpPreviewList preview={preview} />
        </>
      )}
    </Modal>
  );
}

export function LabelsDialog({ occurrenceIds }: { occurrenceIds: string[] }) {
  const { closeDialog, applyMeta } = useApp();
  const [base, setBase] = useState("");
  const [prefix, setPrefix] = useState("");
  const [suffix, setSuffix] = useState("");
  const [numbering, setNumbering] = useState(false);
  const [from, setFrom] = useState(1);
  const [clear, setClear] = useState(false);
  const [rows, setRows] = useState<[string, string][]>([]);
  const op = { type: "SetLabels" as const, occurrence_ids: occurrenceIds, base: base.trim() ? base : null, prefix, suffix, number_from: numbering ? from : null, clear };
  const key = JSON.stringify(op);
  useEffect(() => {
    api()
      .previewLabels(op)
      .then(setRows)
      .catch(() => setRows([]));
  }, [key]); // eslint-disable-line react-hooks/exhaustive-deps
  return (
    <Modal
      title={`Vault labels for ${occurrenceIds.length} sound(s)`}
      onClose={closeDialog}
      wide
      footer={
        <>
          <button onClick={closeDialog}>Cancel</button>
          <button className="primary" onClick={() => void applyMeta(op).then((ok) => ok && closeDialog())}>
            Apply
          </button>
        </>
      }
    >
      <p className="muted small">Vault labels are library metadata. They do not rename programs on the synth and do not add to the write count.</p>
      <label className="toggle">
        <input type="checkbox" checked={clear} onChange={(e) => setClear(e.target.checked)} /> Clear labels (show stored names)
      </label>
      {!clear && (
        <div className="grid2">
          <label>
            Base text (empty = keep stored name)
            <input value={base} onChange={(e) => setBase(e.target.value)} />
          </label>
          <label>
            Prefix
            <input value={prefix} onChange={(e) => setPrefix(e.target.value)} />
          </label>
          <label>
            Suffix
            <input value={suffix} onChange={(e) => setSuffix(e.target.value)} />
          </label>
          <label className="toggle">
            <input type="checkbox" checked={numbering} onChange={(e) => setNumbering(e.target.checked)} /> Number from{" "}
            <input type="number" style={{ width: 60 }} value={from} onChange={(e) => setFrom(Number(e.target.value) || 1)} />
          </label>
        </div>
      )}
      <div className="scroll-box small">
        <table className="simple">
          <tbody>
            {rows.slice(0, 200).map(([a, b], i) => (
              <tr key={i}>
                <td className="muted">{a}</td>
                <td>→ {b}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </Modal>
  );
}

export function ExportSelectionDialog({ items }: { items: { name: string; blob_hash: string; defaultSlot: number }[] }) {
  const { closeDialog, toast, error } = useApp();
  const [dest, setDest] = useState(items.map((i) => i.defaultSlot));
  const dupes = new Set(dest.filter((d, i) => dest.indexOf(d) !== i));
  const go = async () => {
    const path = await api().saveFile("P6-selection.syx");
    if (!path) return;
    try {
      const n = await api().exportSelection(
        items.map((it, i) => ({ slot: dest[i], blob_hash: it.blob_hash })),
        path,
      );
      toast("success", `Exported ${items.length} program(s) (${n} bytes), verified. No MIDI was sent.`);
      closeDialog();
    } catch (e) {
      error(e);
    }
  };
  return (
    <Modal
      title="Export selected programs"
      onClose={closeDialog}
      wide
      footer={
        <>
          <button onClick={closeDialog}>Cancel</button>
          <button className="primary" disabled={dupes.size > 0} onClick={() => void go()}>
            Save .syx…
          </button>
        </>
      }
    >
      <p className="muted small">
        Each program is stored in the file with a user destination (000–499). Sending this file with another utility would write those destinations on the synth.
      </p>
      <div className="row-buttons">
        <button onClick={() => setDest(items.map((_, i) => i))}>Number from 000</button>
      </div>
      <div className="scroll-box">
        <table className="simple">
          <tbody>
            {items.map((it, i) => (
              <tr key={i} className={dupes.has(dest[i]) ? "error" : ""}>
                <td>{it.name}</td>
                <td>
                  <input
                    className="mono"
                    type="number"
                    min={0}
                    max={499}
                    value={dest[i]}
                    onChange={(e) => {
                      const d = [...dest];
                      d[i] = Math.max(0, Math.min(499, Number(e.target.value) || 0));
                      setDest(d);
                    }}
                  />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </Modal>
  );
}

export function ConfirmDialog({ title, body, confirm, onConfirm }: { title: string; body: string; confirm: string; onConfirm: () => void }) {
  const { closeDialog } = useApp();
  return (
    <Modal
      title={title}
      onClose={closeDialog}
      footer={
        <>
          <button onClick={closeDialog}>Cancel</button>
          <button
            className="primary"
            onClick={() => {
              closeDialog();
              onConfirm();
            }}
          >
            {confirm}
          </button>
        </>
      }
    >
      <p>{body}</p>
    </Modal>
  );
}

export function HistoryDialog() {
  const { closeDialog, openDialog, status, error, toast, refreshWorkspace } = useApp();
  const [sessions, setSessions] = useState<WriteSessionRow[]>([]);
  const [buffers, setBuffers] = useState<ProtectedBuffer[]>([]);
  const [snaps, setSnaps] = useState<SnapshotRow[]>([]);
  const [dir, setDir] = useState("");
  const connected = status?.state === "Connected" || status?.state === "Simulator";
  const load = () => {
    void Promise.all([api().listWriteSessions(), api().protectedBuffers(), api().listSnapshots(), api().backupsDir()])
      .then(([s, b, sn, d]) => {
        setSessions(s);
        setBuffers(b);
        setSnaps(sn);
        setDir(d);
      })
      .catch(error);
  };
  useEffect(load, []); // eslint-disable-line react-hooks/exhaustive-deps

  const restoreBuffer = async (id: string) => {
    try {
      await api().restoreProtectedBuffer(id);
      toast("success", "Restored to the edit buffer. The buffer it replaced was saved too.");
      load();
    } catch (e) {
      error(e);
    }
  };
  const exportBuffer = async (b: ProtectedBuffer) => {
    const p = await api().saveFile("P6-edit-buffer.syx");
    if (!p) return;
    try {
      await api().exportEditBuffer(b.blob_hash, p);
      toast("success", "Edit buffer exported.");
    } catch (e) {
      error(e);
    }
  };
  const stageRestore = async (snapshotId: string) => {
    try {
      await api().stageBackupRestore(snapshotId);
      await refreshWorkspace();
      toast("success", "The backup is staged as a separate bank. Review it to write; nothing was sent.");
      closeDialog();
    } catch (e) {
      error(e);
    }
  };

  const unfinished = sessions.filter((s) => ["Interrupted", "NeedsRecovery", "Writing", "Reconciling"].includes(s.status));
  return (
    <Modal title="History & backups" onClose={closeDialog} wide>
      {unfinished.length > 0 && (
        <div className="notice">
          {unfinished.length} unfinished write session(s).{" "}
          <button className="mini" onClick={() => openDialog({ kind: "recovery", session: unfinished[0], report: null })}>
            Recover…
          </button>
        </div>
      )}
      <h3>Write sessions</h3>
      <div className="scroll-box small">
        <table className="simple">
          <thead>
            <tr>
              <th>When</th>
              <th>Status</th>
              <th>Outcome</th>
              <th>Backup</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {sessions.map((s) => (
              <tr key={s.id}>
                <td>{fmtTime(s.created_ms)}</td>
                <td>
                  {s.status}
                  {s.simulator && <span className="badge small sim">SIM</span>}
                </td>
                <td className="muted">{s.outcome ?? s.error ?? ""}</td>
                <td>
                  {s.backup_syx_path && (
                    <button className="mini" onClick={() => void api().reveal(s.backup_syx_path!)}>
                      Show
                    </button>
                  )}
                </td>
                <td>
                  {s.prewrite_snapshot_id && s.backup_syx_path && (
                    <button className="mini" onClick={() => void stageRestore(s.prewrite_snapshot_id!)} title="Stage this complete backup as a separate bank">
                      Restore backup…
                    </button>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <p className="muted small">
        Backups folder: <span className="mono">{dir}</span> (never deleted automatically)
      </p>
      <h3>Protected edit buffers</h3>
      <div className="scroll-box small">
        <table className="simple">
          <tbody>
            {buffers.map((b) => (
              <tr key={b.id}>
                <td>{fmtTime(b.captured_ms)}</td>
                <td>{b.name}</td>
                <td className="muted">{b.restored_ms ? `restored ${fmtTime(b.restored_ms)}` : ""}</td>
                <td>
                  <button className="mini" disabled={!connected} onClick={() => void restoreBuffer(b.id)}>
                    Restore to synth
                  </button>{" "}
                  <button className="mini" onClick={() => void exportBuffer(b)}>
                    Export
                  </button>
                </td>
              </tr>
            ))}
            {buffers.length === 0 && (
              <tr>
                <td className="muted">None yet. The edit buffer is saved automatically before the first audition.</td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
      <h3>Snapshots</h3>
      <div className="scroll-box small">
        <table className="simple">
          <tbody>
            {snaps.map((s) => (
              <tr key={s.id}>
                <td>{fmtTime(s.captured_end_ms)}</td>
                <td>{s.kind}</td>
                <td className="muted">{s.origin}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </Modal>
  );
}

export function DiagnosticsDialog() {
  const { closeDialog, status } = useApp();
  const [rows, setRows] = useState<DiagEntry[]>([]);
  const [err, setErr] = useState<string | null>(null);
  useEffect(() => {
    api()
      .diagnostics()
      .then((r) => setRows(r.slice(-400).reverse()))
      .catch((e) => setErr(e.message ?? String(e)));
  }, []);
  return (
    <Modal title="MIDI diagnostics" onClose={closeDialog} wide>
      <p className="muted small">
        {status?.description ?? "Not connected"} · local only, no payload bytes are logged.
      </p>
      {err && <p className="error">{err}</p>}
      <div className="scroll-box small mono">
        <table className="simple">
          <tbody>
            {rows.map((r, i) => (
              <tr key={i}>
                <td>{new Date(r.at_ms).toLocaleTimeString()}</td>
                <td>{r.direction}</td>
                <td>{r.summary}</td>
                <td>{r.bytes}</td>
                <td>#{r.attempt}</td>
                <td>{r.elapsed_ms} ms</td>
                <td>{r.result}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </Modal>
  );
}
