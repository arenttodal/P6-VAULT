import { useMemo, useState } from "react";
import { api } from "../../api/backend";
import type { ConflictChoice, ReadSessionState, ReconcileSlot, Review, WriteOutcome, WriteSessionRow, InspectReport } from "../../api/types";
import { fmtDuration, fmtTime, slot3 } from "../../lib/format";
import { useApp } from "../../stores/app";
import { Modal } from "../Modal";

const RES_LABEL: Record<string, string> = {
  AdoptHardware: "No staged change — use synth",
  KeepStaged: "Synth unchanged — keep New",
  Agree: "Both agree",
  EmptyStaged: "New slot is empty",
  Conflict: "Conflict",
};

/** Three-way reconciliation after a sync, a drift detection or a recovery inspection. */
export function ReconcileDialog({
  workspaceId,
  snapshotId,
  slots,
  purpose,
  sessionId,
}: {
  workspaceId: string;
  snapshotId: string;
  slots: ReconcileSlot[];
  purpose: "sync" | "drift" | "recovery";
  sessionId?: string;
}) {
  const { closeDialog, refreshWorkspace, workspace, error, toast } = useApp();
  const needs = useMemo(() => slots.filter((s) => s.resolution === "Conflict" || s.resolution === "EmptyStaged"), [slots]);
  const auto = slots.filter((s) => !needs.includes(s));
  const [choices, setChoices] = useState<Record<number, ConflictChoice>>({});
  const [busy, setBusy] = useState(false);
  const all = (c: ConflictChoice) => setChoices(Object.fromEntries(needs.map((s) => [s.slot, c])));
  const complete = needs.every((s) => choices[s.slot]);

  const apply = async () => {
    setBusy(true);
    try {
      if (purpose === "recovery" && sessionId) {
        const r = await api().recoveryRebase(sessionId, choices, "reconciled");
        if (r.kind === "NeedsChoices") throw { message: "More choices are needed." };
      } else {
        const rev = workspace?.id === workspaceId ? workspace.revision : 0;
        await api().applyRebase(workspaceId, rev, snapshotId, choices);
      }
      await refreshWorkspace();
      toast("success", purpose === "drift" ? "Reconciled with the synth. Review again to write." : "Current updated from the synth; your New choices are kept.");
      closeDialog();
    } catch (e) {
      error(e);
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal
      wide
      title={purpose === "drift" ? "The synth changed since Current was captured" : "Reconcile New with the synth"}
      onClose={closeDialog}
      footer={
        <>
          <span className="muted">Nothing is written to the synth here. All versions stay in the library.</span>
          <button onClick={closeDialog}>Later</button>
          <button className="primary" disabled={busy || !complete} onClick={() => void apply()}>
            Apply {needs.length ? `${Object.keys(choices).length}/${needs.length} choices` : ""}
          </button>
        </>
      }
    >
      {purpose === "drift" && <p className="warn">No programs were written. The fresh backup is kept. Decide for each changed slot, then review again.</p>}
      <p>
        {auto.length} slot(s) resolve automatically; {needs.length} need a decision.
      </p>
      {needs.length > 0 && (
        <div className="row-buttons">
          <button onClick={() => all("KeepNew")}>Keep New for all</button>
          <button onClick={() => all("UseSynth")}>Use synth for all</button>
        </div>
      )}
      <div className="scroll-box">
        <table className="simple">
          <thead>
            <tr>
              <th>Slot</th>
              <th>Old Current</th>
              <th>On synth now</th>
              <th>New</th>
              <th>Result</th>
            </tr>
          </thead>
          <tbody>
            {needs.map((s) => (
              <tr key={s.slot}>
                <td className="mono">{slot3(s.slot)}</td>
                <td className="muted">{s.old_name ?? "—"}</td>
                <td>{s.live_name}</td>
                <td>{s.staged_name ?? <i>empty</i>}</td>
                <td>
                  <label>
                    <input type="radio" name={`c${s.slot}`} checked={choices[s.slot] === "KeepNew"} onChange={() => setChoices({ ...choices, [s.slot]: "KeepNew" })} /> Keep New
                  </label>{" "}
                  <label>
                    <input type="radio" name={`c${s.slot}`} checked={choices[s.slot] === "UseSynth"} onChange={() => setChoices({ ...choices, [s.slot]: "UseSynth" })} /> Use synth
                  </label>
                </td>
              </tr>
            ))}
            {auto.slice(0, 200).map((s) => (
              <tr key={s.slot} className="muted">
                <td className="mono">{slot3(s.slot)}</td>
                <td>{s.old_name ?? "—"}</td>
                <td>{s.live_name}</td>
                <td>{s.staged_name ?? "—"}</td>
                <td>{RES_LABEL[s.resolution]}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </Modal>
  );
}

export function SyncPartialDialog({ read }: { read: ReadSessionState }) {
  const { closeDialog, startOp } = useApp();
  return (
    <Modal
      title={`Read ${read.received}/500 programs`}
      onClose={closeDialog}
      footer={
        <>
          <button onClick={closeDialog}>Close</button>
          <button
            className="primary"
            onClick={() => {
              closeDialog();
              void startOp("sync", () => api().syncStart(read.id));
            }}
          >
            Retry {read.missing.length} missing
          </button>
        </>
      }
    >
      <p>
        {read.status === "cancelled" ? "The read was stopped." : "Some slots did not answer."} Missing: <span className="mono">{read.missing.slice(0, 40).map(slot3).join(", ")}</span>
        {read.missing.length > 40 && " …"}
      </p>
      <p className="muted">
        A partial read is kept as a separate source in the library but cannot become Current or a backup. Your previous Current and New are unchanged.
      </p>
    </Modal>
  );
}

export function ReviewDialog({ review }: { review: Review }) {
  const { closeDialog, startOp, error } = useApp();
  const p = review.plan;
  const cancel = async () => {
    try {
      await api().cancelReview(p.session_id);
    } catch (e) {
      error(e);
    }
    closeDialog();
  };
  const write = () => {
    closeDialog();
    void startOp("write", () => api().writeConfirmed(p.session_id, review.plan_hash));
  };
  return (
    <Modal
      wide
      title={`Write ${p.steps.length} program${p.steps.length === 1 ? "" : "s"} to the Prophet-6`}
      onClose={() => void cancel()}
      footer={
        <>
          <span className="muted">
            {review.transport} · estimated {fmtDuration(review.estimated_ms)} incl. final verification
          </span>
          <button onClick={() => void cancel()}>Cancel</button>
          <button className="primary danger" onClick={write}>
            Write {p.steps.length} program{p.steps.length === 1 ? "" : "s"}
          </button>
        </>
      }
    >
      {p.simulator && <p className="badge sim">SIMULATOR — writes go to the simulated synth only</p>}
      <div className="grid2">
        <div>
          <b>Verified backup of all 500 slots</b>
          <div className="mono small path">{p.backup_syx}</div>
          <button className="mini" onClick={() => void api().reveal(p.backup_syx)}>
            Show in Finder
          </button>
        </div>
        <div>
          <b>Per bank</b>
          <div className="mono small">{review.per_bank.map((n, i) => `${i}xx: ${n}`).join(" · ")}</div>
          <div className="small muted">Device: {p.device}</div>
        </div>
      </div>
      <p className="small">
        Only these user slots are written, in order. Each slot is re-read just before writing (stops if it changed), then read back and compared byte-for-byte.
        Stopping or disconnecting mid-way can leave the bank partly updated; the journal and backup make that recoverable. Please don't store programs on the synth or
        use another librarian until this finishes.
      </p>
      <div className="scroll-box">
        <table className="simple">
          <thead>
            <tr>
              <th>Slot</th>
              <th>On synth now</th>
              <th>Will become</th>
            </tr>
          </thead>
          <tbody>
            {p.steps.map((s) => (
              <tr key={s.slot}>
                <td className="mono">{slot3(s.slot)}</td>
                <td className="muted">{s.before_name}</td>
                <td>{s.desired_name}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </Modal>
  );
}

export function WriteResultDialog({ outcome }: { outcome: WriteOutcome }) {
  const { closeDialog, openDialog, error } = useApp();
  const ok = outcome.status === "Completed";
  const toRecovery = async () => {
    try {
      const sessions = await api().listWriteSessions();
      const s = sessions.find((x) => x.id === outcome.session_id);
      if (s) openDialog({ kind: "recovery", session: s, report: null });
    } catch (e) {
      error(e);
    }
  };
  return (
    <Modal
      title={ok ? "Bank written and verified" : "Write stopped"}
      onClose={closeDialog}
      footer={
        <>
          {!ok && <button onClick={() => void toRecovery()}>Inspect & recover…</button>}
          <button className="primary" onClick={closeDialog}>
            OK
          </button>
        </>
      }
    >
      {ok ? (
        <p>
          {outcome.verified} program(s) updated; all 500 slots were read back and match New exactly ({fmtTime(Date.now())}). Current now reflects the synth.
        </p>
      ) : (
        <>
          <p className="warn">{outcome.first_error}</p>
          <ul>
            <li>Verified: {outcome.verified}</li>
            <li>Not attempted: {outcome.not_attempted}</li>
            <li>Failed / uncertain: {outcome.failed_or_uncertain}</li>
            {outcome.final_mismatch_slots.length > 0 && <li>Final read disagrees at: {outcome.final_mismatch_slots.map(slot3).join(", ")}</li>}
          </ul>
          <p className="muted">New, all sources and the pre-write backup are kept. Nothing further is written until you inspect and confirm again.</p>
        </>
      )}
    </Modal>
  );
}

export function RecoveryDialog({ session, report }: { session: WriteSessionRow; report: InspectReport | null }) {
  const { closeDialog, startOp, status, error, toast, refreshWorkspace, openDialog, workspace } = useApp();
  const connected = status?.state === "Connected" || status?.state === "Simulator";
  const [busy, setBusy] = useState(false);

  const cont = async (outcome: string) => {
    if (!report) return;
    setBusy(true);
    try {
      const r = await api().recoveryRebase(session.id, {}, outcome);
      await refreshWorkspace();
      if (r.kind === "NeedsChoices") {
        openDialog({ kind: "reconcile", workspaceId: r.workspace_id, snapshotId: r.live_snapshot_id, slots: r.slots, purpose: "recovery", sessionId: session.id });
        return;
      }
      toast("success", outcome === "continued" ? "Ready: choose Review to write the remaining changes." : "Current now matches the synth. Your New is kept.");
      closeDialog();
    } catch (e) {
      error(e);
    } finally {
      setBusy(false);
    }
  };
  const restore = async () => {
    setBusy(true);
    try {
      const r = await api().recoveryRestore(session.id);
      await refreshWorkspace();
      if (r.kind === "RestoreWorkspace")
        toast(
          "success",
          `A restoration bank was staged (${r.restored_slots.length} slot(s))${r.conflicts.length ? `; ${r.conflicts.length} slot(s) hold other content and were left as-is` : ""}. Review it to write.`,
        );
      closeDialog();
    } catch (e) {
      error(e);
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal wide title="An earlier write did not finish" onClose={closeDialog}>
      <p>
        Session from {fmtTime(session.created_ms)} · status <b>{session.status}</b> {session.error && `· ${session.error}`}
      </p>
      <p className="muted small">
        Nothing is sent automatically. Backup: <span className="mono">{session.backup_syx_path}</span>{" "}
        {session.backup_syx_path && (
          <button className="mini" onClick={() => void api().reveal(session.backup_syx_path!)}>
            Show
          </button>
        )}
      </p>
      {!report ? (
        <div className="stack">
          <p>First, read the synth to see what actually happened. {workspace ? "" : ""}</p>
          <button className="primary" disabled={!connected || busy} onClick={() => void startOp("inspect", () => api().inspectSession(session.id))}>
            {connected ? "Inspect interrupted write (reads all 500 slots)" : "Connect the synth to inspect"}
          </button>
        </div>
      ) : (
        <div className="stack">
          <p>
            Planned slots: {report.slots.length} · already written: <b>{report.matches_desired}</b> · still original: <b>{report.matches_before}</b> · other content:{" "}
            <b>{report.conflicts}</b> · no reply: <b>{report.unknown}</b>
          </p>
          {!report.live_snapshot_id && <p className="warn">The read was incomplete; further writes stay unavailable. Retry the inspection.</p>}
          <div className="scroll-box small">
            <table className="simple">
              <tbody>
                {report.slots.map((s) => (
                  <tr key={s.slot}>
                    <td className="mono">{slot3(s.slot)}</td>
                    <td>{s.before_name}</td>
                    <td>→ {s.desired_name}</td>
                    <td>{s.observation}</td>
                    <td className="muted">{s.journal_state}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <div className="row-buttons">
            <button className="primary" disabled={busy || !report.live_snapshot_id} onClick={() => void cont("continued")}>
              Continue deployment…
            </button>
            <button disabled={busy || !report.live_snapshot_id} onClick={() => void restore()}>
              Restore affected slots…
            </button>
            <button disabled={busy || !report.live_snapshot_id} onClick={() => void cont("kept hardware")}>
              Keep the synth as it is
            </button>
          </div>
          <p className="muted small">Every option ends in the normal review with a fresh backup and your explicit confirmation.</p>
        </div>
      )}
    </Modal>
  );
}

export function ProtectDialog({ pending }: { pending: { hash: string; label: string } | null }) {
  const { closeDialog, set, status, error, auditionHash } = useApp();
  const [busy, setBusy] = useState(false);
  const go = async () => {
    setBusy(true);
    try {
      await api().protectEditBuffer();
      set({ status: status ? { ...status, buffer_protected: true } : status });
      closeDialog();
      if (pending) await auditionHash(pending.hash, pending.label, true);
    } catch (e) {
      error(e);
    } finally {
      setBusy(false);
    }
  };
  return (
    <Modal
      title="Protect the current sound first"
      onClose={closeDialog}
      footer={
        <>
          <button onClick={closeDialog}>Cancel</button>
          <button className="primary" disabled={busy} onClick={() => void go()}>
            {busy ? "Saving…" : "Save buffer & audition"}
          </button>
        </>
      }
    >
      <p>Audition replaces the Prophet's edit buffer, including unsaved edits. Your current buffer will be saved first. Stored programs are unchanged.</p>
      <p className="muted small">Restore it any time from History & backups → Protected edit buffers.</p>
    </Modal>
  );
}

export function CloseBlockedDialog({ opKind, opId }: { opKind: string; opId: string }) {
  const { closeDialog } = useApp();
  return (
    <Modal
      title="An operation is running"
      onClose={closeDialog}
      footer={
        <>
          <button className="primary" onClick={closeDialog}>
            Keep running
          </button>
          <button
            onClick={() => {
              void api().stopOperation(opId);
              closeDialog();
            }}
          >
            {opKind === "write" ? "Stop after current program" : "Stop"}
          </button>
        </>
      }
    >
      <p>
        {opKind === "write"
          ? "Programs are being written. Quitting now could leave the bank partly updated. Stop safely first, then quit."
          : "The synth is being read. Stop it first, then quit."}
      </p>
    </Modal>
  );
}
