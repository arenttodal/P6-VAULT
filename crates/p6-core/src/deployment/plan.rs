//! Review preparation: fresh full backup, drift detection, frozen plan and confirmation.

use super::backup::{write_backup, BackupInfo};
use super::permit::ConfirmedWritePermit;
use super::sync::read_bank;
use super::*;
use crate::device::{Device, TransportKind};
use crate::storage::workspace::ReconcileSlot;
use crate::storage::Vault;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlanStep {
    pub slot: u16,
    pub expected_before: String,
    pub desired: String,
    pub before_name: String,
    pub desired_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FrozenPlan {
    pub session_id: String,
    pub workspace_id: String,
    pub workspace_revision: i64,
    pub baseline_snapshot_id: String,
    pub prewrite_snapshot_id: String,
    pub epoch: u64,
    pub device: String,
    pub simulator: bool,
    pub steps: Vec<PlanStep>,
    /// Desired hash for each of the 500 slots (the complete target bank).
    pub target: Vec<String>,
    pub backup_syx: String,
    pub backup_manifest: String,
    pub backup_hash: String,
    pub created_ms: i64,
}

impl FrozenPlan {
    pub fn hash(&self) -> String {
        hex::encode(Sha256::digest(serde_json::to_vec(self).unwrap()))
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Review {
    pub plan: FrozenPlan,
    pub plan_hash: String,
    pub per_bank: [usize; 5],
    pub estimated_ms: u64,
    pub transport: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind")]
pub enum PrepareOutcome {
    Ready(Box<Review>),
    NoChanges { session_id: String, backup: Option<BackupInfo> },
    Drift { session_id: String, live_snapshot_id: String, slots: Vec<ReconcileSlot> },
}

/// Prepare a review. Transmits only read requests (05). Never stores.
pub fn prepare(dev: &mut Device, vault: &Mutex<Vault>, ws: &str, cancel: &AtomicBool, progress: &dyn Fn(Progress)) -> DResult<PrepareOutcome> {
    let simulator = dev.kind() == TransportKind::Simulator;
    let session_id = crate::util::new_id();
    let (rev, baseline, staged) = {
        let mut v = vault.lock().unwrap();
        let view = v.workspace_view(ws)?;
        if !view.writable_baseline {
            return Err(DeployError::Invalid("Current is not a hardware snapshot yet. Sync with the synth and reconcile first.".into()));
        }
        if view.empty_count > 0 {
            return Err(DeployError::Vault(crate::storage::VaultError::IncompleteBank(view.empty_count)));
        }
        if !simulator {
            let gate = v.hardware_gate()?;
            if !gate.passed && view.changed_count != 1 {
                return Err(DeployError::HardwareGate { done: gate.verified_single_slot_sessions, required: gate.required, changed: view.changed_count });
            }
        }
        let staged = v.staged_payloads(ws)?;
        if staged.iter().any(|p| p.is_none()) {
            return Err(DeployError::Invalid("New has empty slots".into()));
        }
        let baseline = view.baseline.unwrap().id;
        v.checkpoint(ws, "before write review")?;
        v.create_write_session(&session_id, ws, view.revision, dev.epoch(), &dev.description(), simulator, Some(&baseline), None)?;
        (view.revision, baseline, staged)
    };
    let fail = |status: &str, msg: &str| {
        let _ = vault.lock().unwrap().set_session_status(&session_id, status, Some(msg), Some(msg));
    };

    // 1. Fresh complete live snapshot.
    let read = match read_bank(dev, vault, None, "prewrite", "prewrite", cancel, progress) {
        Ok(r) => r,
        Err(e) => {
            fail("CancelledBeforeWrite", &e.to_string());
            return Err(e);
        }
    };
    let Some(live) = read.snapshot_id.clone() else {
        fail("CancelledBeforeWrite", "backup read incomplete");
        return Err(DeployError::IncompleteBank { missing: read.missing.len() });
    };
    let mut v = vault.lock().unwrap();
    v.set_session_field(&session_id, "prewrite_snapshot_id", &live)?;

    // 2. Drift check against Current.
    let base_cells = v.snapshot_cells(&baseline)?;
    let live_cells = v.snapshot_cells(&live)?;
    if base_cells.iter().zip(&live_cells).any(|(a, b)| a.blob_hash != b.blob_hash) {
        let slots = v.reconcile_preview(ws, &live)?;
        v.set_session_status(&session_id, "CancelledBeforeWrite", Some("hardware drift"), None)?;
        return Ok(PrepareOutcome::Drift { session_id, live_snapshot_id: live, slots });
    }

    // 3. Verified backup.
    let backup = match write_backup(&v, &session_id, &live, serde_json::to_value(&dev.profile).unwrap(), simulator) {
        Ok(b) => b,
        Err(e) => {
            v.set_session_status(&session_id, "CancelledBeforeWrite", Some("backup failed"), Some(&e.to_string()))?;
            return Err(e);
        }
    };
    v.set_session_field(&session_id, "backup_syx_path", &backup.syx_path.to_string_lossy())?;
    v.set_session_field(&session_id, "backup_manifest_path", &backup.manifest_path.to_string_lossy())?;
    v.set_session_field(&session_id, "backup_hash", &backup.file_hash)?;

    // 4. Diff + freeze.
    let view = v.workspace_view(ws)?;
    if view.revision != rev {
        v.set_session_status(&session_id, "CancelledBeforeWrite", Some("workspace changed"), None)?;
        return Err(DeployError::Vault(crate::storage::VaultError::RevisionConflict { current: view.revision }));
    }
    let target: Vec<String> = staged.iter().map(|p| p.as_ref().unwrap().exact_hash()).collect();
    let steps: Vec<PlanStep> = view
        .slots
        .iter()
        .filter(|s| live_cells[s.slot].blob_hash != target[s.slot])
        .map(|s| PlanStep {
            slot: s.slot as u16,
            expected_before: live_cells[s.slot].blob_hash.clone(),
            desired: target[s.slot].clone(),
            before_name: s.current.as_ref().map(|c| c.name.clone()).unwrap_or_default(),
            desired_name: s.new.as_ref().map(|c| c.name.clone()).unwrap_or_default(),
        })
        .collect();
    if steps.is_empty() {
        v.set_session_status(&session_id, "CancelledBeforeWrite", Some("no changes"), None)?;
        return Ok(PrepareOutcome::NoChanges { session_id, backup: Some(backup) });
    }
    let plan = FrozenPlan {
        session_id: session_id.clone(),
        workspace_id: ws.to_string(),
        workspace_revision: rev,
        baseline_snapshot_id: baseline,
        prewrite_snapshot_id: live,
        epoch: dev.epoch(),
        device: dev.description(),
        simulator,
        steps,
        target,
        backup_syx: backup.syx_path.to_string_lossy().into(),
        backup_manifest: backup.manifest_path.to_string_lossy().into(),
        backup_hash: backup.file_hash.clone(),
        created_ms: crate::util::now_ms(),
    };
    let plan_hash = plan.hash();
    let rows: Vec<(u16, String, String)> = plan.steps.iter().map(|s| (s.slot, s.expected_before.clone(), s.desired.clone())).collect();
    if let Err(e) = v.freeze_plan(&session_id, &serde_json::to_string(&plan).unwrap(), &plan_hash, &rows) {
        let _ = v.set_session_status(&session_id, "CancelledBeforeWrite", Some("journal failed"), Some(&e.to_string()));
        return Err(DeployError::JournalFailed(e.to_string()));
    }
    let mut per_bank = [0usize; 5];
    for s in &plan.steps {
        per_bank[(s.slot / 100) as usize] += 1;
    }
    let estimated_ms = dev.profile.estimate_write(plan.steps.len()).as_millis() as u64 + dev.profile.estimate_read(500).as_millis() as u64;
    Ok(PrepareOutcome::Ready(Box::new(Review { plan, plan_hash, per_bank, estimated_ms, transport: format!("{:?}", dev.kind()) })))
}

/// Explicit user confirmation of exactly this plan. Returns the only permit type that can
/// authorize stored writes.
pub fn confirm(vault: &Mutex<Vault>, session_id: &str, plan_hash: &str, current_epoch: u64) -> DResult<ConfirmedWritePermit> {
    let v = vault.lock().unwrap();
    let s = v.write_session(session_id)?;
    if s.status != "Ready" {
        return Err(DeployError::InvalidPermit(format!("session is {}", s.status)));
    }
    let (json, stored_hash) = v.plan_json(session_id)?;
    let plan: FrozenPlan = serde_json::from_str(&json).map_err(|e| DeployError::InvalidPermit(e.to_string()))?;
    if stored_hash != plan_hash || plan.hash() != plan_hash {
        return Err(DeployError::InvalidPermit("plan changed".into()));
    }
    if plan.epoch != current_epoch {
        return Err(DeployError::InvalidPermit("the connection changed since review".into()));
    }
    let rev = v.workspace_revision(&plan.workspace_id)?;
    if rev != plan.workspace_revision {
        return Err(DeployError::InvalidPermit("New changed since review".into()));
    }
    Ok(ConfirmedWritePermit::new(session_id.into(), plan_hash.into(), rev, current_epoch))
}

pub fn cancel_review(vault: &Mutex<Vault>, session_id: &str) -> DResult<()> {
    let mut v = vault.lock().unwrap();
    let s = v.write_session(session_id)?;
    if s.status == "Ready" || s.status == "Preparing" {
        v.set_session_status(session_id, "CancelledBeforeWrite", Some("cancelled by user"), None)?;
    }
    Ok(())
}

pub fn load_plan(vault: &Vault, session_id: &str) -> DResult<FrozenPlan> {
    let (json, _) = vault.plan_json(session_id)?;
    serde_json::from_str(&json).map_err(|e| DeployError::Invalid(e.to_string()))
}
