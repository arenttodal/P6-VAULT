//! Durable write-session journal. Every transition commits before the next MIDI action.

use super::*;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct WriteSessionRow {
    pub id: String,
    pub workspace_id: String,
    pub workspace_revision: i64,
    pub epoch: i64,
    pub device: String,
    pub simulator: bool,
    pub plan_hash: String,
    pub status: String,
    pub outcome: Option<String>,
    pub error: Option<String>,
    pub baseline_snapshot_id: Option<String>,
    pub prewrite_snapshot_id: Option<String>,
    pub final_snapshot_id: Option<String>,
    pub backup_syx_path: Option<String>,
    pub backup_manifest_path: Option<String>,
    pub parent_session_id: Option<String>,
    pub created_ms: i64,
    pub updated_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct WriteStepRow {
    pub slot: u16,
    pub expected_before_hash: String,
    pub desired_hash: String,
    pub state: String,
    pub attempts: i64,
    pub readback_hash: Option<String>,
    pub error: Option<String>,
}

const SESSION_COLS: &str = "id,workspace_id,workspace_revision,epoch,device,simulator,plan_hash,status,outcome,error,baseline_snapshot_id,prewrite_snapshot_id,final_snapshot_id,backup_syx_path,backup_manifest_path,parent_session_id,created_ms,updated_ms";

fn session_row(r: &rusqlite::Row) -> rusqlite::Result<WriteSessionRow> {
    Ok(WriteSessionRow {
        id: r.get(0)?,
        workspace_id: r.get(1)?,
        workspace_revision: r.get(2)?,
        epoch: r.get(3)?,
        device: r.get(4)?,
        simulator: r.get(5)?,
        plan_hash: r.get(6)?,
        status: r.get(7)?,
        outcome: r.get(8)?,
        error: r.get(9)?,
        baseline_snapshot_id: r.get(10)?,
        prewrite_snapshot_id: r.get(11)?,
        final_snapshot_id: r.get(12)?,
        backup_syx_path: r.get(13)?,
        backup_manifest_path: r.get(14)?,
        parent_session_id: r.get(15)?,
        created_ms: r.get(16)?,
        updated_ms: r.get(17)?,
    })
}

/// Hardware validation gate (docs/HARDWARE-TESTS.md): until the owner has completed a
/// verified single-slot write AND a verified single-slot restoration on real hardware,
/// real (non-simulator) deployments are limited to exactly one changed slot.
#[derive(Debug, Clone, Serialize)]
pub struct HardwareGate {
    pub passed: bool,
    pub verified_single_slot_sessions: usize,
    pub required: usize,
}

pub const GATE_REQUIRED_SESSIONS: usize = 2;

impl Vault {
    pub fn hardware_gate(&self) -> VResult<HardwareGate> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM write_sessions ws WHERE ws.simulator = 0 AND ws.status = 'Completed' AND ws.outcome = 'verified'
               AND (SELECT COUNT(*) FROM write_steps st WHERE st.session_id = ws.id) = 1",
            [],
            |r| r.get(0),
        )?;
        let n = n as usize;
        Ok(HardwareGate { passed: n >= GATE_REQUIRED_SESSIONS, verified_single_slot_sessions: n, required: GATE_REQUIRED_SESSIONS })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn create_write_session(
        &mut self,
        id: &str,
        ws: &str,
        rev: i64,
        epoch: u64,
        device: &str,
        simulator: bool,
        baseline: Option<&str>,
        parent: Option<&str>,
    ) -> VResult<()> {
        let now = now_ms();
        self.conn.execute(
            "INSERT INTO write_sessions(id,workspace_id,workspace_revision,epoch,device,simulator,plan_json,plan_hash,baseline_snapshot_id,parent_session_id,status,created_ms,updated_ms)
             VALUES(?1,?2,?3,?4,?5,?6,'{}','',?7,?8,'Preparing',?9,?9)",
            params![id, ws, rev, epoch as i64, device, simulator, baseline, parent, now],
        )?;
        Ok(())
    }

    pub(crate) fn set_session_status(&mut self, id: &str, status: &str, outcome: Option<&str>, error: Option<&str>) -> VResult<()> {
        self.conn.execute(
            "UPDATE write_sessions SET status=?2, outcome=COALESCE(?3,outcome), error=COALESCE(?4,error), updated_ms=?5 WHERE id=?1",
            params![id, status, outcome, error, now_ms()],
        )?;
        Ok(())
    }

    pub(crate) fn set_session_field(&mut self, id: &str, field: &str, value: &str) -> VResult<()> {
        const ALLOWED: &[&str] = &["prewrite_snapshot_id", "final_snapshot_id", "backup_syx_path", "backup_manifest_path", "backup_hash"];
        if !ALLOWED.contains(&field) {
            return Err(VaultError::Invalid(format!("field {field}")));
        }
        self.conn.execute(&format!("UPDATE write_sessions SET {field}=?2, updated_ms=?3 WHERE id=?1"), params![id, value, now_ms()])?;
        Ok(())
    }

    /// Freeze the plan: store plan JSON + hash and all steps as Planned, status Ready.
    pub(crate) fn freeze_plan(&mut self, id: &str, plan_json: &str, plan_hash: &str, steps: &[(u16, String, String)]) -> VResult<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "UPDATE write_sessions SET plan_json=?2, plan_hash=?3, status='Ready', updated_ms=?4 WHERE id=?1",
            params![id, plan_json, plan_hash, now_ms()],
        )?;
        for (i, (slot, before, desired)) in steps.iter().enumerate() {
            tx.execute(
                "INSERT INTO write_steps(session_id,slot,ord,expected_before_hash,desired_hash,state,attempts,updated_ms) VALUES(?1,?2,?3,?4,?5,'Planned',0,?6)",
                params![id, slot, i as i64, before, desired, now_ms()],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn plan_json(&self, id: &str) -> VResult<(String, String)> {
        Ok(self.conn.query_row("SELECT plan_json, plan_hash FROM write_sessions WHERE id=?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))?)
    }

    /// Record a step transition durably (synchronous=FULL commit).
    pub(crate) fn set_step(&mut self, id: &str, slot: u16, state: &str, bump_attempt: bool, readback: Option<&str>, error: Option<&str>) -> VResult<()> {
        let n = self.conn.execute(
            "UPDATE write_steps SET state=?3, attempts=attempts+?4, readback_hash=COALESCE(?5,readback_hash), error=?6, updated_ms=?7 WHERE session_id=?1 AND slot=?2",
            params![id, slot, state, bump_attempt as i64, readback, error, now_ms()],
        )?;
        if n != 1 {
            return Err(VaultError::Database("journal step not found".into()));
        }
        Ok(())
    }

    pub fn write_session(&self, id: &str) -> VResult<WriteSessionRow> {
        Ok(self.conn.query_row(&format!("SELECT {SESSION_COLS} FROM write_sessions WHERE id=?1"), [id], session_row)?)
    }

    pub fn write_steps(&self, id: &str) -> VResult<Vec<WriteStepRow>> {
        let mut st = self
            .conn
            .prepare("SELECT slot,expected_before_hash,desired_hash,state,attempts,readback_hash,error FROM write_steps WHERE session_id=?1 ORDER BY ord")?;
        let rows = st
            .query_map([id], |r| {
                Ok(WriteStepRow {
                    slot: r.get(0)?,
                    expected_before_hash: r.get(1)?,
                    desired_hash: r.get(2)?,
                    state: r.get(3)?,
                    attempts: r.get(4)?,
                    readback_hash: r.get(5)?,
                    error: r.get(6)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }

    pub fn list_write_sessions(&self) -> VResult<Vec<WriteSessionRow>> {
        let mut st = self.conn.prepare(&format!("SELECT {SESSION_COLS} FROM write_sessions ORDER BY created_ms DESC"))?;
        let rows = st.query_map([], session_row)?.collect::<Result<_, _>>()?;
        Ok(rows)
    }

    /// Called once at startup. Sessions that never reached a stored send are closed as
    /// CancelledBeforeWrite; sessions that were Writing/Reconciling become Interrupted.
    /// No MIDI is sent.
    pub fn startup_recovery_scan(&mut self) -> VResult<Vec<WriteSessionRow>> {
        let sessions = self.list_write_sessions()?;
        for s in &sessions {
            match s.status.as_str() {
                "Preparing" | "Ready" => {
                    let sent: bool =
                        self.conn.query_row("SELECT EXISTS(SELECT 1 FROM write_steps WHERE session_id=?1 AND state<>'Planned')", [&s.id], |r| r.get(0))?;
                    if sent {
                        self.set_session_status(&s.id, "Interrupted", None, Some("app stopped during session"))?;
                    } else {
                        self.set_session_status(&s.id, "CancelledBeforeWrite", Some("app restarted before confirmation"), None)?;
                    }
                }
                "Writing" | "Reconciling" => {
                    // A SendIntent without a later state is uncertain: the program may have been stored.
                    self.conn.execute(
                        "UPDATE write_steps SET state='Uncertain', error='app stopped after send intent' WHERE session_id=?1 AND state IN ('SendIntent','SentUnverified')",
                        [&s.id],
                    )?;
                    self.set_session_status(&s.id, "Interrupted", None, Some("app stopped during deployment"))?;
                }
                _ => {}
            }
        }
        self.unfinished_sessions()
    }

    pub fn unfinished_sessions(&self) -> VResult<Vec<WriteSessionRow>> {
        Ok(self
            .list_write_sessions()?
            .into_iter()
            .filter(|s| matches!(s.status.as_str(), "Interrupted" | "NeedsRecovery" | "Writing" | "Reconciling"))
            .collect())
    }

    /// Store an observed (e.g. unexpected/drifted) payload so it is never lost.
    pub fn preserve_observation(&mut self, label: &str, slot: u16, p: &Payload) -> VResult<String> {
        let tx = self.conn.transaction()?;
        let (hash, _) = insert_blob(&tx, p)?;
        let sid = insert_source(&tx, label, "partial_read", None, None, None, None)?;
        insert_occurrence(&tx, &sid, &hash, 0, None, "program", Some(slot), None, false)?;
        tx.commit()?;
        Ok(hash)
    }
}
