//! Interrupted-session inspection and recovery actions. Nothing here sends a stored
//! write; every action ends in the normal reviewed deployment flow.

use super::plan::load_plan;
use super::sync::read_bank;
use super::*;
use crate::device::Device;
use crate::storage::workspace::ReconcileSlot;
use crate::storage::Vault;
use crate::workspace::reconcile::ConflictChoice;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub enum Observation {
    MatchesDesired,
    MatchesBefore,
    Neither,
    NoReply,
}

#[derive(Debug, Clone, Serialize)]
pub struct InspectedSlot {
    pub slot: u16,
    pub journal_state: String,
    pub observation: Observation,
    pub before_name: String,
    pub desired_name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct InspectReport {
    pub session_id: String,
    pub live_snapshot_id: Option<String>,
    pub slots: Vec<InspectedSlot>,
    pub matches_desired: usize,
    pub matches_before: usize,
    pub conflicts: usize,
    pub unknown: usize,
}

/// Fresh full-bank read, then classify each planned destination.
pub fn inspect(dev: &mut Device, vault: &Mutex<Vault>, session_id: &str, cancel: &AtomicBool, progress: &dyn Fn(Progress)) -> DResult<InspectReport> {
    let (plan, steps) = {
        let v = vault.lock().unwrap();
        (load_plan(&v, session_id)?, v.write_steps(session_id)?)
    };
    let read = read_bank(dev, vault, None, "recovery inspection", "live", cancel, progress)?;
    let v = vault.lock().unwrap();
    let observed: HashMap<u16, String> = match &read.snapshot_id {
        Some(s) => v.snapshot_cells(s)?.into_iter().enumerate().map(|(i, c)| (i as u16, c.blob_hash)).collect(),
        None => {
            let cells = v.source_programs(&read.source_id)?;
            cells.into_iter().filter_map(|(_, a, h)| a.map(|a| (a, h))).collect()
        }
    };
    let mut slots = Vec::new();
    for (step, ps) in steps.iter().zip(&plan.steps) {
        let obs = match observed.get(&step.slot) {
            None => Observation::NoReply,
            Some(h) if *h == step.desired_hash => Observation::MatchesDesired,
            Some(h) if *h == step.expected_before_hash => Observation::MatchesBefore,
            Some(_) => Observation::Neither,
        };
        slots.push(InspectedSlot {
            slot: step.slot,
            journal_state: step.state.clone(),
            observation: obs,
            before_name: ps.before_name.clone(),
            desired_name: ps.desired_name.clone(),
        });
    }
    let count = |o| slots.iter().filter(|s| s.observation == o).count();
    Ok(InspectReport {
        session_id: session_id.into(),
        live_snapshot_id: read.snapshot_id,
        matches_desired: count(Observation::MatchesDesired),
        matches_before: count(Observation::MatchesBefore),
        conflicts: count(Observation::Neither),
        unknown: count(Observation::NoReply),
        slots,
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind")]
pub enum RecoveryResult {
    /// Workspace rebased onto the fresh snapshot; ready for a new review.
    Rebased { workspace_id: String, revision: i64 },
    /// Conflicts need an explicit Keep New / Use Synth choice first.
    NeedsChoices { workspace_id: String, live_snapshot_id: String, slots: Vec<ReconcileSlot> },
    /// A separate restoration workspace was created.
    RestoreWorkspace { workspace_id: String, restored_slots: Vec<u16>, conflicts: Vec<u16> },
}

fn require_complete(report: &InspectReport) -> DResult<String> {
    report.live_snapshot_id.clone().ok_or(DeployError::IncompleteBank { missing: report.unknown.max(1) })
}

/// Continue deployment / Keep hardware as it is: rebase the original workspace onto the
/// fresh snapshot (three-way), then close the session. Continue then runs a normal review.
pub fn rebase_after_inspection(
    vault: &Mutex<Vault>,
    report: &InspectReport,
    choices: &HashMap<usize, ConflictChoice>,
    outcome: &str,
) -> DResult<RecoveryResult> {
    let live = require_complete(report)?;
    let mut v = vault.lock().unwrap();
    let plan = load_plan(&v, &report.session_id)?;
    let ws = plan.workspace_id.clone();
    let preview = v.reconcile_preview(&ws, &live)?;
    let needs: Vec<ReconcileSlot> = preview
        .into_iter()
        .filter(|s| matches!(s.resolution, crate::workspace::reconcile::SlotResolution::Conflict | crate::workspace::reconcile::SlotResolution::EmptyStaged))
        .collect();
    if needs.iter().any(|s| !choices.contains_key(&s.slot)) {
        return Ok(RecoveryResult::NeedsChoices { workspace_id: ws, live_snapshot_id: live, slots: needs });
    }
    let rev = v.workspace_revision(&ws)?;
    v.set_session_status(&report.session_id, "Closed", Some(outcome), None)?;
    let rev = v.apply_rebase(&ws, rev, &live, choices)?;
    Ok(RecoveryResult::Rebased { workspace_id: ws, revision: rev })
}

/// Restore affected destinations: a separate workspace whose baseline is the fresh
/// snapshot and whose New puts the pre-write payloads back at planned destinations that
/// currently hold the desired (or pre-write) program. Slots holding a third version are
/// left as-is and reported as conflicts (the newer version is preserved).
pub fn restore_affected(vault: &Mutex<Vault>, report: &InspectReport) -> DResult<RecoveryResult> {
    let live = require_complete(report)?;
    let mut v = vault.lock().unwrap();
    let plan = load_plan(&v, &report.session_id)?;
    let live_cells = v.snapshot_cells(&live)?;
    let pre = v.snapshot_cells(&plan.prewrite_snapshot_id)?;
    let mut cells: Vec<Option<(String, Option<String>)>> = live_cells.iter().map(|c| Some((c.blob_hash.clone(), c.occurrence_id.clone()))).collect();
    let mut restored = Vec::new();
    let mut conflicts = Vec::new();
    for s in &report.slots {
        match s.observation {
            Observation::MatchesDesired => {
                let p = &pre[s.slot as usize];
                cells[s.slot as usize] = Some((p.blob_hash.clone(), p.occurrence_id.clone()));
                restored.push(s.slot);
            }
            Observation::MatchesBefore => {}
            Observation::Neither | Observation::NoReply => conflicts.push(s.slot),
        }
    }
    let ws = v.create_workspace_with_cells(&format!("Restore after write {}", &report.session_id[..8]), Some(&live), cells)?;
    v.set_session_status(&report.session_id, "Closed", Some("restore staged"), None)?;
    Ok(RecoveryResult::RestoreWorkspace { workspace_id: ws, restored_slots: restored, conflicts })
}

/// Restore an entire backup: a separate workspace whose New is the backup bank and
/// whose baseline is the given writable baseline. Follows the normal deployment flow.
pub fn stage_backup_restore(vault: &Mutex<Vault>, backup_snapshot_id: &str, baseline_snapshot_id: &str) -> DResult<String> {
    let mut v = vault.lock().unwrap();
    let cells = v.snapshot_cells(backup_snapshot_id)?.into_iter().map(|c| Some((c.blob_hash, c.occurrence_id))).collect();
    Ok(v.create_workspace_with_cells("Restore of backup", Some(baseline_snapshot_id), cells)?)
}
