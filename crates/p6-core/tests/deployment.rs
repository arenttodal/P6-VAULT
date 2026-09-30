//! End-to-end deployment tests against the simulated Prophet-6.

use p6_core::deployment::plan::{cancel_review, confirm, prepare, PrepareOutcome, Review};
use p6_core::deployment::recovery::{
    inspect, rebase_after_inspection, restore_affected, Observation, RecoveryResult,
};
use p6_core::deployment::sync::read_bank;
use p6_core::deployment::writer::execute;
use p6_core::deployment::{DeployError, Progress};
use p6_core::device::{Device, TransportProfile};
use p6_core::protocol::messages::is_stored_write;
use p6_core::protocol::payload::synthetic_payload;
use p6_core::simulator::SimulatedP6;
use p6_core::storage::workspace::WorkspaceOp;
use p6_core::storage::Vault;
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

struct Env {
    _dir: tempfile::TempDir,
    vault: Mutex<Vault>,
    sim_state: Arc<Mutex<p6_core::simulator::SimState>>,
    ctl: Arc<Mutex<p6_core::simulator::SimControl>>,
    sim: SimulatedP6,
    dev: Device,
    ws: String,
}

fn np(_: Progress) {}

fn stores(e: &Env) -> Vec<u16> {
    e.ctl.lock().unwrap().stores.clone()
}

fn setup() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let vault = Mutex::new(Vault::open(dir.path()).unwrap());
    let sim = SimulatedP6::new();
    let sim_state = sim.state.clone();
    let ctl = sim.control.clone();
    let sim2 = sim.reconnect();
    let mut dev = Device::new(Box::new(sim2), TransportProfile::simulator(), 1);
    let never = AtomicBool::new(false);
    let r = read_bank(&mut dev, &vault, None, "sync", "live", &never, &np).unwrap();
    let snap = r.snapshot_id.expect("complete");
    let ws = vault
        .lock()
        .unwrap()
        .create_workspace("Main", Some(&snap), None)
        .unwrap();
    Env {
        _dir: dir,
        vault,
        sim_state,
        ctl,
        sim,
        dev,
        ws,
    }
}

fn stage_moves(e: &Env) {
    let mut v = e.vault.lock().unwrap();
    let rev = v.workspace_revision(&e.ws).unwrap();
    // Swap 0-1 with 150-151 and move 3 to 499 (shifts 4..499 down).
    let rev = v
        .apply_op(
            &e.ws,
            rev,
            &WorkspaceOp::SwapRanges {
                a: 0,
                b: 150,
                len: 2,
            },
        )
        .unwrap();
    v.apply_op(
        &e.ws,
        rev,
        &WorkspaceOp::MoveToSlot {
            slots: vec![3],
            target: 499,
        },
    )
    .unwrap();
}

fn ready(o: PrepareOutcome) -> Review {
    match o {
        PrepareOutcome::Ready(r) => *r,
        other => panic!("expected Ready, got {other:?}"),
    }
}

fn target_hashes(e: &Env) -> Vec<String> {
    e.vault
        .lock()
        .unwrap()
        .staged_payloads(&e.ws)
        .unwrap()
        .into_iter()
        .map(|p| p.unwrap().exact_hash())
        .collect()
}

#[test]
fn happy_path_writes_only_changed_and_verifies() {
    let mut e = setup();
    stage_moves(&e);
    let target = target_hashes(&e);
    let never = AtomicBool::new(false);
    let review = ready(prepare(&mut e.dev, &e.vault, &e.ws, &never, &np).unwrap());
    // 2+2 swapped + 496 shifted (3..499) = 500? slots 3..499 all change; 0,1,150,151 change.
    let n = review.plan.steps.len();
    assert!(n > 0);
    assert!(stores(&e).is_empty(), "preparation must not store");
    // New is frozen during review
    {
        let mut v = e.vault.lock().unwrap();
        let rev = v.workspace_revision(&e.ws).unwrap();
        assert!(v
            .apply_op(&e.ws, rev, &WorkspaceOp::ResetToBaseline)
            .is_err());
    }
    // backup exists and re-parses to the pre-write bank
    let bytes = std::fs::read(&review.plan.backup_syx).unwrap();
    assert_eq!(bytes.len(), 589000);
    let permit = confirm(&e.vault, &review.plan.session_id, &review.plan_hash, 1).unwrap();
    let out = execute(&mut e.dev, &e.vault, permit, &never, &np).unwrap();
    assert_eq!(out.status, "Completed", "{out:?}");
    assert_eq!(out.verified, n);
    let bank: Vec<String> = e.sim_state.lock().unwrap().programs[..500]
        .iter()
        .map(|p| p.exact_hash())
        .collect();
    assert_eq!(bank, target);
    let mut s = stores(&e);
    s.sort();
    let mut planned: Vec<u16> = review.plan.steps.iter().map(|s| s.slot).collect();
    planned.sort();
    assert_eq!(s, planned, "only changed slots written, each once");
    let view = e.vault.lock().unwrap().workspace_view(&e.ws).unwrap();
    assert_eq!(view.changed_count, 0);
    assert_eq!(view.baseline.unwrap().kind, "post_write");
    assert!(e.sim.user_bank().len() == 500);
}

#[test]
fn confirm_rejects_wrong_epoch_and_hash() {
    let mut e = setup();
    stage_moves(&e);
    let never = AtomicBool::new(false);
    let review = ready(prepare(&mut e.dev, &e.vault, &e.ws, &never, &np).unwrap());
    assert!(matches!(
        confirm(&e.vault, &review.plan.session_id, &review.plan_hash, 2),
        Err(DeployError::InvalidPermit(_))
    ));
    assert!(matches!(
        confirm(&e.vault, &review.plan.session_id, "bad", 1),
        Err(DeployError::InvalidPermit(_))
    ));
    cancel_review(&e.vault, &review.plan.session_id).unwrap();
    assert!(confirm(&e.vault, &review.plan.session_id, &review.plan_hash, 1).is_err());
    assert!(stores(&e).is_empty());
    // after cancel, editing works again
    let mut v = e.vault.lock().unwrap();
    let rev = v.workspace_revision(&e.ws).unwrap();
    v.apply_op(&e.ws, rev, &WorkspaceOp::ResetToBaseline)
        .unwrap();
}

#[test]
fn drift_blocks_plan_and_reconciles() {
    let mut e = setup();
    stage_moves(&e);
    e.sim
        .external_store(250, synthetic_payload(77777, "Owner Edit"));
    e.sim
        .external_store(0, synthetic_payload(88888, "Owner Edit 2"));
    let never = AtomicBool::new(false);
    let out = prepare(&mut e.dev, &e.vault, &e.ws, &never, &np).unwrap();
    let PrepareOutcome::Drift {
        live_snapshot_id,
        slots,
        ..
    } = out
    else {
        panic!("expected drift")
    };
    assert!(stores(&e).is_empty());
    // slot 0 was staged (swap) and changed on hardware -> conflict; slot 250 shifted by move -> conflict too.
    let conflicts: Vec<usize> = slots
        .iter()
        .filter(|s| s.resolution == p6_core::workspace::reconcile::SlotResolution::Conflict)
        .map(|s| s.slot)
        .collect();
    assert!(conflicts.contains(&0));
    let mut v = e.vault.lock().unwrap();
    let rev = v.workspace_revision(&e.ws).unwrap();
    assert!(v
        .apply_rebase(&e.ws, rev, &live_snapshot_id, &HashMap::new())
        .is_err());
    let choices = conflicts
        .iter()
        .map(|&s| (s, p6_core::workspace::reconcile::ConflictChoice::KeepNew))
        .collect();
    v.apply_rebase(&e.ws, rev, &live_snapshot_id, &choices)
        .unwrap();
    drop(v);
    let review = ready(prepare(&mut e.dev, &e.vault, &e.ws, &never, &np).unwrap());
    assert!(review.plan.steps.iter().any(|s| s.slot == 0));
}

#[test]
fn incomplete_backup_blocks_writes() {
    let mut e = setup();
    stage_moves(&e);
    e.ctl.lock().unwrap().drop_replies = 3; // one slot never answers (3 attempts)
    let never = AtomicBool::new(false);
    let r = prepare(&mut e.dev, &e.vault, &e.ws, &never, &np);
    assert!(
        matches!(r, Err(DeployError::IncompleteBank { missing: 1 })),
        "{r:?}"
    );
    assert!(stores(&e).is_empty());
}

#[test]
fn mismatch_stops_and_later_slots_untouched() {
    let mut e = setup();
    stage_moves(&e);
    let never = AtomicBool::new(false);
    let review = ready(prepare(&mut e.dev, &e.vault, &e.ws, &never, &np).unwrap());
    let bad = review.plan.steps[1].slot;
    e.ctl.lock().unwrap().corrupt_store_slots.insert(bad);
    let permit = confirm(&e.vault, &review.plan.session_id, &review.plan_hash, 1).unwrap();
    let r = execute(&mut e.dev, &e.vault, permit, &never, &np).unwrap();
    assert_eq!(r.status, "NeedsRecovery");
    assert_eq!(r.verified, 1);
    assert_eq!(stores(&e), vec![review.plan.steps[0].slot, bad]);
    let steps = e
        .vault
        .lock()
        .unwrap()
        .write_steps(&review.plan.session_id)
        .unwrap();
    assert_eq!(steps[1].state, "Failed");
    assert!(steps[2..].iter().all(|s| s.state == "Planned"));
    // Baseline not advanced
    assert_eq!(
        e.vault
            .lock()
            .unwrap()
            .workspace_view(&e.ws)
            .unwrap()
            .baseline
            .unwrap()
            .kind,
        "live"
    );
}

#[test]
fn write_retry_when_store_does_not_land() {
    let mut e = setup();
    stage_moves(&e);
    let never = AtomicBool::new(false);
    let review = ready(prepare(&mut e.dev, &e.vault, &e.ws, &never, &np).unwrap());
    e.ctl.lock().unwrap().ignore_next_stores = 2;
    let permit = confirm(&e.vault, &review.plan.session_id, &review.plan_hash, 1).unwrap();
    let r = execute(&mut e.dev, &e.vault, permit, &never, &np).unwrap();
    assert_eq!(r.status, "Completed");
    let steps = e
        .vault
        .lock()
        .unwrap()
        .write_steps(&review.plan.session_id)
        .unwrap();
    assert_eq!(steps[0].attempts, 3);
}

#[test]
fn persistent_non_landing_write_fails_after_three_attempts() {
    let mut e = setup();
    stage_moves(&e);
    let never = AtomicBool::new(false);
    let review = ready(prepare(&mut e.dev, &e.vault, &e.ws, &never, &np).unwrap());
    e.ctl
        .lock()
        .unwrap()
        .ignore_store_slots
        .insert(review.plan.steps[0].slot);
    let permit = confirm(&e.vault, &review.plan.session_id, &review.plan_hash, 1).unwrap();
    let r = execute(&mut e.dev, &e.vault, permit, &never, &np).unwrap();
    assert_eq!(r.status, "NeedsRecovery");
    assert_eq!(stores(&e).len(), 3);
}

#[test]
fn drift_during_write_stops_before_that_slot() {
    let mut e = setup();
    stage_moves(&e);
    let never = AtomicBool::new(false);
    let review = ready(prepare(&mut e.dev, &e.vault, &e.ws, &never, &np).unwrap());
    let victim = review.plan.steps[2].slot;
    let foreign = synthetic_payload(4242, "Newer Work");
    e.sim.external_store(victim, foreign.clone());
    let permit = confirm(&e.vault, &review.plan.session_id, &review.plan_hash, 1).unwrap();
    let r = execute(&mut e.dev, &e.vault, permit, &never, &np).unwrap();
    assert_eq!(r.status, "NeedsRecovery");
    assert_eq!(stores(&e).len(), 2);
    assert_eq!(
        e.sim_state.lock().unwrap().programs[victim as usize],
        foreign,
        "newer work preserved"
    );
    // and it is preserved in the library
    let rows = e.vault.lock().unwrap().list_occurrences().unwrap();
    assert!(rows.iter().any(|r| r.exact_hash == foreign.exact_hash()));
}

#[test]
fn stop_after_current_program() {
    let mut e = setup();
    stage_moves(&e);
    let never = AtomicBool::new(false);
    let review = ready(prepare(&mut e.dev, &e.vault, &e.ws, &never, &np).unwrap());
    let stop = AtomicBool::new(false);
    let permit = confirm(&e.vault, &review.plan.session_id, &review.plan_hash, 1).unwrap();
    let stop_ref = &stop;
    let prog = move |p: Progress| {
        if p.phase == "write" && p.done == 3 {
            stop_ref.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    };
    let r = execute(&mut e.dev, &e.vault, permit, &stop, &prog).unwrap();
    assert_eq!(r.status, "Interrupted");
    assert_eq!(r.verified, 4);
    assert_eq!(stores(&e).len(), 4);
}

#[test]
fn disconnect_then_inspect_and_restore() {
    let mut e = setup();
    stage_moves(&e);
    let never = AtomicBool::new(false);
    let review = ready(prepare(&mut e.dev, &e.vault, &e.ws, &never, &np).unwrap());
    e.ctl.lock().unwrap().disconnect_after_stores = Some(3);
    let permit = confirm(&e.vault, &review.plan.session_id, &review.plan_hash, 1).unwrap();
    let r = execute(&mut e.dev, &e.vault, permit, &never, &np).unwrap();
    assert_eq!(r.status, "NeedsRecovery", "{r:?}");
    assert_eq!(r.verified, 3);

    // "Restart": startup scan sends no MIDI.
    let sent_before = e.ctl.lock().unwrap().sent.len();
    let unfinished = e.vault.lock().unwrap().startup_recovery_scan().unwrap();
    assert_eq!(unfinished.len(), 1);
    assert_eq!(e.ctl.lock().unwrap().sent.len(), sent_before);

    // Reconnect with a new epoch and inspect.
    let mut dev2 = Device::new(
        Box::new(e.sim.reconnect()),
        TransportProfile::simulator(),
        2,
    );
    let rep = inspect(&mut dev2, &e.vault, &review.plan.session_id, &never, &np).unwrap();
    assert_eq!(rep.matches_desired, 3);
    assert_eq!(rep.slots[3].observation, Observation::MatchesBefore);
    let res = restore_affected(&e.vault, &rep).unwrap();
    let RecoveryResult::RestoreWorkspace {
        workspace_id,
        restored_slots,
        conflicts,
    } = res
    else {
        panic!()
    };
    assert_eq!(restored_slots.len(), 3);
    assert!(conflicts.is_empty());
    // Deploy the restoration via the normal guarded flow.
    let rv = ready(prepare(&mut dev2, &e.vault, &workspace_id, &never, &np).unwrap());
    assert_eq!(rv.plan.steps.len(), 3);
    let permit = confirm(&e.vault, &rv.plan.session_id, &rv.plan_hash, 2).unwrap();
    let out = execute(&mut dev2, &e.vault, permit, &never, &np).unwrap();
    assert_eq!(out.status, "Completed");
    // Hardware equals the original pre-write bank again.
    let pre = e
        .vault
        .lock()
        .unwrap()
        .snapshot_payloads(&review.plan.prewrite_snapshot_id)
        .unwrap();
    assert_eq!(e.sim.user_bank(), pre);
}

#[test]
fn continue_after_interruption() {
    let mut e = setup();
    stage_moves(&e);
    let target = target_hashes(&e);
    let never = AtomicBool::new(false);
    let review = ready(prepare(&mut e.dev, &e.vault, &e.ws, &never, &np).unwrap());
    e.ctl.lock().unwrap().disconnect_after_stores = Some(5);
    let permit = confirm(&e.vault, &review.plan.session_id, &review.plan_hash, 1).unwrap();
    execute(&mut e.dev, &e.vault, permit, &never, &np).unwrap();
    let mut dev2 = Device::new(
        Box::new(e.sim.reconnect()),
        TransportProfile::simulator(),
        2,
    );
    let rep = inspect(&mut dev2, &e.vault, &review.plan.session_id, &never, &np).unwrap();
    let res = rebase_after_inspection(&e.vault, &rep, &HashMap::new(), "continued").unwrap();
    assert!(matches!(res, RecoveryResult::Rebased { .. }), "{res:?}");
    let rv = ready(prepare(&mut dev2, &e.vault, &e.ws, &never, &np).unwrap());
    assert_eq!(rv.plan.steps.len(), review.plan.steps.len() - 5);
    let permit = confirm(&e.vault, &rv.plan.session_id, &rv.plan_hash, 2).unwrap();
    assert_eq!(
        execute(&mut dev2, &e.vault, permit, &never, &np)
            .unwrap()
            .status,
        "Completed"
    );
    let bank: Vec<String> = e.sim.user_bank().iter().map(|p| p.exact_hash()).collect();
    assert_eq!(bank, target);
}

#[test]
fn crash_after_send_intent_is_uncertain() {
    let mut e = setup();
    stage_moves(&e);
    let never = AtomicBool::new(false);
    let review = ready(prepare(&mut e.dev, &e.vault, &e.ws, &never, &np).unwrap());
    // Simulate a crash: journal says Writing with a SendIntent step and nothing after.
    let dir = e._dir.path().to_path_buf();
    {
        let c = rusqlite::Connection::open(dir.join("vault.sqlite")).unwrap();
        c.execute(
            "UPDATE write_sessions SET status='Writing' WHERE id=?1",
            [&review.plan.session_id],
        )
        .unwrap();
        c.execute(
            "UPDATE write_steps SET state='SendIntent', attempts=1 WHERE session_id=?1 AND ord=0",
            [&review.plan.session_id],
        )
        .unwrap();
    }
    let mut v = Vault::open(&dir).unwrap();
    let un = v.startup_recovery_scan().unwrap();
    assert_eq!(un[0].status, "Interrupted");
    assert_eq!(
        v.write_steps(&review.plan.session_id).unwrap()[0].state,
        "Uncertain"
    );
    assert!(stores(&e).is_empty());
}

#[test]
fn no_stored_writes_outside_write_engine() {
    let mut e = setup();
    stage_moves(&e);
    let p = synthetic_payload(5, "Audition");
    e.dev.load_edit_buffer(&p).unwrap();
    e.dev.read_edit_buffer().unwrap();
    {
        let mut v = e.vault.lock().unwrap();
        let rev = v.workspace_revision(&e.ws).unwrap();
        let rev = v.undo(&e.ws, rev).unwrap();
        v.redo(&e.ws, rev).unwrap();
    }
    assert!(e
        .ctl
        .lock()
        .unwrap()
        .sent
        .iter()
        .all(|f| !is_stored_write(f)));
}
