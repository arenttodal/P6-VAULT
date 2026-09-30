//! The WriteEngine: the only code path that transmits stored-program data (command 02).
//!
//! Per changed slot, ascending: fresh read -> compare expected-before -> durable
//! SendIntent -> transmit -> drain -> exact read-back -> durable Verified. Any
//! doubt stops the session; later slots are never sent.

use super::permit::ConfirmedWritePermit;
use super::plan::{load_plan, FrozenPlan};
use super::sync::read_bank;
use super::*;
use crate::device::Device;
use crate::protocol::payload::Payload;
use crate::slot::UserSlot;
use crate::storage::Vault;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

#[derive(Debug, Clone, Serialize)]
pub struct WriteOutcome {
    pub session_id: String,
    pub status: String,
    pub outcome: String,
    pub verified: usize,
    pub not_attempted: usize,
    pub failed_or_uncertain: usize,
    pub first_error: Option<String>,
    pub final_mismatch_slots: Vec<u16>,
    pub final_snapshot_id: Option<String>,
}

enum StepResult {
    Verified,
    Stop(String, &'static str),
}

pub fn execute(
    dev: &mut Device,
    vault: &Mutex<Vault>,
    permit: ConfirmedWritePermit,
    stop: &AtomicBool,
    progress: &dyn Fn(Progress),
) -> DResult<WriteOutcome> {
    let plan: FrozenPlan = {
        let mut v = vault.lock().unwrap();
        let s = v.write_session(permit.session_id())?;
        if s.status != "Ready" {
            return Err(DeployError::InvalidPermit(format!(
                "session is {}",
                s.status
            )));
        }
        let plan = load_plan(&v, permit.session_id())?;
        if plan.hash() != permit.plan_hash() || s.plan_hash != permit.plan_hash() {
            return Err(DeployError::InvalidPermit("plan hash mismatch".into()));
        }
        if dev.epoch() != permit.epoch() || plan.epoch != permit.epoch() {
            return Err(DeployError::InvalidPermit("connection changed".into()));
        }
        if v.workspace_revision(&plan.workspace_id)? != permit.workspace_revision() {
            return Err(DeployError::InvalidPermit("New changed".into()));
        }
        // Backup must still exist and match.
        let bytes = std::fs::read(&plan.backup_syx)
            .map_err(|e| DeployError::BackupFailed(e.to_string()))?;
        if crate::library::import::file_hash(&bytes) != plan.backup_hash {
            return Err(DeployError::BackupFailed(
                "backup file no longer matches".into(),
            ));
        }
        v.set_session_status(permit.session_id(), "Writing", None, None)
            .map_err(|e| DeployError::JournalFailed(e.to_string()))?;
        plan
    };
    let sid = plan.session_id.clone();
    let total = plan.steps.len();
    let mut verified = 0;
    let mut stop_err: Option<(String, &'static str)> = None;

    for (i, step) in plan.steps.iter().enumerate() {
        if stop.load(Ordering::Relaxed) {
            stop_err = Some(("stopped by user".into(), "Interrupted"));
            break;
        }
        progress(Progress::new("write", i, total, Some(step.slot)));
        match write_one(
            dev,
            vault,
            &permit,
            &plan,
            step.slot,
            &step.expected_before,
            &step.desired,
        ) {
            Ok(StepResult::Verified) => verified += 1,
            Ok(StepResult::Stop(msg, status)) => {
                stop_err = Some((msg, status));
                break;
            }
            Err(e) => {
                stop_err = Some((e.to_string(), "NeedsRecovery"));
                break;
            }
        }
    }
    progress(Progress::new("write", verified, total, None));

    let steps = vault.lock().unwrap().write_steps(&sid)?;
    let failed = steps
        .iter()
        .filter(|s| {
            matches!(
                s.state.as_str(),
                "Failed" | "Uncertain" | "SendIntent" | "SentUnverified"
            )
        })
        .count();
    let not_attempted = steps.iter().filter(|s| s.state == "Planned").count();

    if let Some((msg, status)) = stop_err {
        let status = if failed > 0 { "NeedsRecovery" } else { status };
        vault
            .lock()
            .unwrap()
            .set_session_status(&sid, status, Some("stopped"), Some(&msg))?;
        return Ok(WriteOutcome {
            session_id: sid,
            status: status.into(),
            outcome: "stopped".into(),
            verified,
            not_attempted,
            failed_or_uncertain: failed,
            first_error: Some(msg),
            final_mismatch_slots: vec![],
            final_snapshot_id: None,
        });
    }

    // Final reconciliation: read all 500 and compare with the full frozen target.
    vault
        .lock()
        .unwrap()
        .set_session_status(&sid, "Reconciling", None, None)?;
    let never = AtomicBool::new(false);
    let read = read_bank(
        dev,
        vault,
        None,
        "post-write verification",
        "post_write",
        &never,
        progress,
    );
    let mut v = vault.lock().unwrap();
    let base = WriteOutcome {
        session_id: sid.clone(),
        status: String::new(),
        outcome: String::new(),
        verified,
        not_attempted: 0,
        failed_or_uncertain: 0,
        first_error: None,
        final_mismatch_slots: vec![],
        final_snapshot_id: None,
    };
    let snap = match read {
        Ok(r) if r.snapshot_id.is_some() => r.snapshot_id.unwrap(),
        Ok(r) => {
            let msg = format!(
                "{verified} writes verified; full-bank verification pending ({} slot(s) unread)",
                r.missing.len()
            );
            v.set_session_status(
                &sid,
                "NeedsRecovery",
                Some("final_read_incomplete"),
                Some(&msg),
            )?;
            return Ok(WriteOutcome {
                status: "NeedsRecovery".into(),
                outcome: "final_read_incomplete".into(),
                first_error: Some(msg),
                ..base
            });
        }
        Err(e) => {
            let msg = format!("{verified} writes verified; full-bank verification pending ({e})");
            v.set_session_status(
                &sid,
                "NeedsRecovery",
                Some("final_read_incomplete"),
                Some(&msg),
            )?;
            return Ok(WriteOutcome {
                status: "NeedsRecovery".into(),
                outcome: "final_read_incomplete".into(),
                first_error: Some(msg),
                ..base
            });
        }
    };
    v.set_session_field(&sid, "final_snapshot_id", &snap)?;
    let cells = v.snapshot_cells(&snap)?;
    let mismatch: Vec<u16> = (0..500u16)
        .filter(|&i| cells[i as usize].blob_hash != plan.target[i as usize])
        .collect();
    if !mismatch.is_empty() {
        let msg = format!(
            "final read disagrees with the target at {} slot(s)",
            mismatch.len()
        );
        v.set_session_status(&sid, "NeedsRecovery", Some("final_mismatch"), Some(&msg))?;
        return Ok(WriteOutcome {
            status: "NeedsRecovery".into(),
            outcome: "final_mismatch".into(),
            first_error: Some(msg),
            final_mismatch_slots: mismatch,
            final_snapshot_id: Some(snap),
            ..base
        });
    }
    v.advance_baseline(&plan.workspace_id, &snap)?;
    v.set_session_status(&sid, "Completed", Some("verified"), None)?;
    Ok(WriteOutcome {
        status: "Completed".into(),
        outcome: "verified".into(),
        final_snapshot_id: Some(snap),
        ..base
    })
}

fn journal(
    vault: &Mutex<Vault>,
    sid: &str,
    slot: u16,
    state: &str,
    bump: bool,
    readback: Option<&str>,
    err: Option<&str>,
) -> DResult<()> {
    vault
        .lock()
        .unwrap()
        .set_step(sid, slot, state, bump, readback, err)
        .map_err(|e| DeployError::JournalFailed(e.to_string()))
}

fn write_one(
    dev: &mut Device,
    vault: &Mutex<Vault>,
    permit: &ConfirmedWritePermit,
    plan: &FrozenPlan,
    slot_n: u16,
    expected_before: &str,
    desired_hash: &str,
) -> DResult<StepResult> {
    let sid = &plan.session_id;
    let slot = UserSlot::new(slot_n)
        .ok_or_else(|| DeployError::Invalid(format!("slot {slot_n} is not a user slot")))?;
    let desired: Payload = vault.lock().unwrap().payload(desired_hash)?;
    if desired.exact_hash() != desired_hash {
        return Err(DeployError::Invalid("desired payload hash mismatch".into()));
    }
    let max_attempts = dev.profile.max_write_attempts.max(1);
    let mut attempt = 0;
    loop {
        // Fresh observation immediately before writing.
        let observed = match dev.read_program(slot.address(), None) {
            Ok(p) => p,
            Err(e) => {
                let state = if attempt == 0 { "Planned" } else { "Uncertain" };
                journal(vault, sid, slot_n, state, false, None, Some(&e.to_string()))?;
                return Ok(StepResult::Stop(
                    format!("could not read slot {slot} before writing: {e}"),
                    "Interrupted",
                ));
            }
        };
        let oh = observed.exact_hash();
        if oh == desired_hash {
            journal(vault, sid, slot_n, "Verified", false, Some(&oh), None)?;
            return Ok(StepResult::Verified);
        }
        if oh != expected_before {
            vault.lock().unwrap().preserve_observation(
                &format!("Unexpected content at {slot} during write"),
                slot_n,
                &observed,
            )?;
            let state = if attempt == 0 { "Failed" } else { "Uncertain" };
            journal(
                vault,
                sid,
                slot_n,
                state,
                false,
                Some(&oh),
                Some("hardware drift: slot changed since backup"),
            )?;
            return Ok(StepResult::Stop(
                format!(
                    "slot {slot} changed on the synth since the backup; nothing was written there"
                ),
                "NeedsRecovery",
            ));
        }
        if attempt >= max_attempts {
            journal(
                vault,
                sid,
                slot_n,
                "Failed",
                false,
                Some(&oh),
                Some("write did not take effect after retries"),
            )?;
            return Ok(StepResult::Stop(
                format!("slot {slot} did not accept the write after {attempt} attempt(s)"),
                "NeedsRecovery",
            ));
        }
        attempt += 1;
        // Durable intent BEFORE the send: a crash after this point is uncertain.
        journal(vault, sid, slot_n, "SendIntent", true, None, None)?;
        if let Err(e) = dev.transmit_stored_program(permit, slot, &desired) {
            journal(
                vault,
                sid,
                slot_n,
                "Uncertain",
                false,
                None,
                Some(&e.to_string()),
            )?;
            return Ok(StepResult::Stop(
                format!("send failed at {slot}: {e}"),
                "NeedsRecovery",
            ));
        }
        journal(vault, sid, slot_n, "SentUnverified", false, None, None)?;
        // Never accept a reply that was buffered before the write.
        if let Err(e) = dev.drain(Duration::from_millis(dev.profile.inter_request_ms.max(20))) {
            journal(
                vault,
                sid,
                slot_n,
                "Uncertain",
                false,
                None,
                Some(&e.to_string()),
            )?;
            return Ok(StepResult::Stop(
                format!("connection lost after writing {slot}: {e}"),
                "NeedsRecovery",
            ));
        }
        match dev.read_program(slot.address(), None) {
            Ok(back) => {
                let bh = back.exact_hash();
                if bh == desired_hash {
                    journal(vault, sid, slot_n, "Verified", false, Some(&bh), None)?;
                    return Ok(StepResult::Verified);
                }
                if bh == expected_before {
                    // Write did not land; loop re-reads and may retry within the plan.
                    continue;
                }
                vault.lock().unwrap().preserve_observation(
                    &format!("Read-back mismatch at {slot}"),
                    slot_n,
                    &back,
                )?;
                let d = back.diff(&desired);
                journal(
                    vault,
                    sid,
                    slot_n,
                    "Failed",
                    false,
                    Some(&bh),
                    Some(&format!(
                        "read-back differs in {} byte(s), first at {:?}",
                        d.count, d.first_offsets
                    )),
                )?;
                return Err(DeployError::VerificationMismatch {
                    slot: slot_n,
                    differing: d.count,
                });
            }
            Err(e) => {
                journal(
                    vault,
                    sid,
                    slot_n,
                    "Uncertain",
                    false,
                    None,
                    Some(&e.to_string()),
                )?;
                return Ok(StepResult::Stop(
                    format!("could not verify {slot}: {e}"),
                    "NeedsRecovery",
                ));
            }
        }
    }
}
