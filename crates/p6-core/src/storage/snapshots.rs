//! Read sessions and immutable 500-slot snapshots.

use super::*;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct SnapshotRow {
    pub id: String,
    pub kind: String,
    pub origin: String,
    pub device: Option<String>,
    pub captured_start_ms: i64,
    pub captured_end_ms: i64,
    pub sealed: bool,
    pub source_id: Option<String>,
}

/// One cell of a snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SnapCell {
    pub blob_hash: String,
    pub occurrence_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReadSessionState {
    pub id: String,
    pub source_id: String,
    pub received: usize,
    pub missing: Vec<u16>,
    pub status: String,
    pub snapshot_id: Option<String>,
}

impl Vault {
    pub fn begin_read_session(&mut self, purpose: &str, device: &str, epoch: u64) -> VResult<String> {
        let tx = self.conn.transaction()?;
        let name = format!("Hardware read {} ({purpose})", crate::util::file_timestamp(now_ms()));
        let sid = insert_source(&tx, &name, "partial_read", None, None, None, Some(device))?;
        let id = new_id();
        tx.execute(
            "INSERT INTO read_sessions(id,purpose,status,device,epoch,started_ms,source_id) VALUES(?1,?2,'running',?3,?4,?5,?6)",
            params![id, purpose, device, epoch as i64, now_ms(), sid],
        )?;
        tx.commit()?;
        Ok(id)
    }

    /// Persist one received slot immediately.
    pub fn record_read_slot(&mut self, session: &str, slot: u16, p: &Payload) -> VResult<()> {
        let tx = self.conn.transaction()?;
        let sid: String = tx.query_row("SELECT source_id FROM read_sessions WHERE id=?1", [session], |r| r.get(0))?;
        let exists: bool =
            tx.query_row("SELECT EXISTS(SELECT 1 FROM source_occurrences WHERE source_id=?1 AND orig_address=?2)", params![sid, slot], |r| r.get(0))?;
        if exists {
            return Err(VaultError::Invalid(format!("slot {slot:03} already recorded in this read")));
        }
        let (hash, _) = insert_blob(&tx, p)?;
        insert_occurrence(&tx, &sid, &hash, slot as usize, None, "program", Some(slot), None, false)?;
        tx.execute("UPDATE read_sessions SET received=received+1 WHERE id=?1", [session])?;
        tx.commit()?;
        Ok(())
    }

    pub fn read_session_state(&self, session: &str) -> VResult<ReadSessionState> {
        let (sid, status, snap): (String, String, Option<String>) =
            self.conn
                .query_row("SELECT source_id, status, snapshot_id FROM read_sessions WHERE id=?1", [session], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        let mut st = self.conn.prepare("SELECT orig_address FROM source_occurrences WHERE source_id=?1 AND orig_address < 500")?;
        let have: std::collections::HashSet<u16> = st.query_map([&sid], |r| r.get(0))?.collect::<Result<_, _>>()?;
        let missing: Vec<u16> = (0..500).filter(|s| !have.contains(s)).collect();
        Ok(ReadSessionState { id: session.into(), source_id: sid, received: have.len(), missing, status, snapshot_id: snap })
    }

    /// Finish a read. Seals a live snapshot only if all 500 slots were received.
    pub fn finish_read_session(&mut self, session: &str, cancelled: bool, kind: &str, origin: &str) -> VResult<ReadSessionState> {
        let state = self.read_session_state(session)?;
        let (device, started): (Option<String>, i64) =
            self.conn.query_row("SELECT device, started_ms FROM read_sessions WHERE id=?1", [session], |r| Ok((r.get(0)?, r.get(1)?)))?;
        let tx = self.conn.transaction()?;
        let status;
        let mut snap_id = None;
        if state.missing.is_empty() {
            let sid = new_id();
            tx.execute(
                "INSERT INTO snapshots(id,kind,origin,device,captured_start_ms,captured_end_ms,sealed,source_id) VALUES(?1,?2,?3,?4,?5,?6,0,?7)",
                params![sid, kind, origin, device, started, now_ms(), state.source_id],
            )?;
            tx.execute(
                "INSERT INTO snapshot_slots(snapshot_id,slot,blob_hash,occurrence_id) SELECT ?1, orig_address, blob_hash, id FROM source_occurrences WHERE source_id=?2 AND orig_address < 500",
                params![sid, state.source_id],
            )?;
            seal(&tx, &sid)?;
            let src_kind = match kind {
                "post_write" => "post_write",
                "prewrite" => "backup",
                _ => "live_snapshot",
            };
            tx.execute("UPDATE sources SET kind=?2 WHERE id=?1", params![state.source_id, src_kind])?;
            status = "complete";
            snap_id = Some(sid);
        } else {
            status = if cancelled { "cancelled" } else { "partial" };
        }
        tx.execute(
            "UPDATE read_sessions SET status=?2, finished_ms=?3, missing=?4, snapshot_id=?5 WHERE id=?1",
            params![session, status, now_ms(), serde_json::to_string(&state.missing).unwrap(), snap_id],
        )?;
        tx.commit()?;
        self.read_session_state(session)
    }

    /// Build a sealed snapshot from an imported source that is a complete, unambiguous 000-499 bank.
    pub fn snapshot_from_source(&mut self, source_id: &str, kind: &str) -> VResult<String> {
        let progs = self.source_programs(source_id)?;
        let mut cells: Vec<Option<(String, String)>> = vec![None; 500];
        for (occ, addr, hash) in progs {
            if let Some(a) = addr.filter(|a| *a < 500) {
                if cells[a as usize].is_some() {
                    return Err(VaultError::Invalid(format!("source has more than one program for slot {a:03}; choose explicitly")));
                }
                cells[a as usize] = Some((hash, occ));
            }
        }
        let missing = cells.iter().filter(|c| c.is_none()).count();
        if missing > 0 {
            return Err(VaultError::IncompleteBank(missing));
        }
        let name: String = self.conn.query_row("SELECT name FROM sources WHERE id=?1", [source_id], |r| r.get(0))?;
        let tx = self.conn.transaction()?;
        let sid = new_id();
        let now = now_ms();
        tx.execute(
            "INSERT INTO snapshots(id,kind,origin,device,captured_start_ms,captured_end_ms,sealed,source_id) VALUES(?1,?2,?3,NULL,?4,?4,0,?5)",
            params![sid, kind, format!("Imported from {name}"), now, source_id],
        )?;
        for (i, c) in cells.into_iter().enumerate() {
            let (h, o) = c.unwrap();
            tx.execute("INSERT INTO snapshot_slots(snapshot_id,slot,blob_hash,occurrence_id) VALUES(?1,?2,?3,?4)", params![sid, i as i64, h, o])?;
        }
        seal(&tx, &sid)?;
        tx.commit()?;
        Ok(sid)
    }

    pub fn snapshot(&self, id: &str) -> VResult<SnapshotRow> {
        Ok(self.conn.query_row("SELECT id,kind,origin,device,captured_start_ms,captured_end_ms,sealed,source_id FROM snapshots WHERE id=?1", [id], snap_row)?)
    }

    pub fn list_snapshots(&self) -> VResult<Vec<SnapshotRow>> {
        let mut st = self
            .conn
            .prepare("SELECT id,kind,origin,device,captured_start_ms,captured_end_ms,sealed,source_id FROM snapshots ORDER BY captured_end_ms DESC")?;
        let rows = st.query_map([], snap_row)?.collect::<Result<_, _>>()?;
        Ok(rows)
    }

    pub fn snapshot_cells(&self, id: &str) -> VResult<Vec<SnapCell>> {
        let mut st = self.conn.prepare("SELECT blob_hash, occurrence_id FROM snapshot_slots WHERE snapshot_id=?1 ORDER BY slot")?;
        let cells: Vec<SnapCell> = st.query_map([id], |r| Ok(SnapCell { blob_hash: r.get(0)?, occurrence_id: r.get(1)? }))?.collect::<Result<_, _>>()?;
        if cells.len() != 500 {
            return Err(VaultError::IncompleteBank(500 - cells.len()));
        }
        Ok(cells)
    }

    pub fn snapshot_payloads(&self, id: &str) -> VResult<Vec<Payload>> {
        self.snapshot_cells(id)?.iter().map(|c| self.payload(&c.blob_hash)).collect()
    }
}

fn snap_row(r: &rusqlite::Row) -> rusqlite::Result<SnapshotRow> {
    Ok(SnapshotRow {
        id: r.get(0)?,
        kind: r.get(1)?,
        origin: r.get(2)?,
        device: r.get(3)?,
        captured_start_ms: r.get(4)?,
        captured_end_ms: r.get(5)?,
        sealed: r.get(6)?,
        source_id: r.get(7)?,
    })
}

pub(crate) fn seal(tx: &Transaction, id: &str) -> VResult<()> {
    let n: i64 = tx.query_row("SELECT COUNT(*) FROM snapshot_slots WHERE snapshot_id=?1", [id], |r| r.get(0))?;
    if n != 500 {
        return Err(VaultError::IncompleteBank(500 - n as usize));
    }
    tx.execute("UPDATE snapshots SET sealed=1 WHERE id=?1", [id])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::testutil::vault;
    use crate::protocol::payload::synthetic_payload;

    #[test]
    fn partial_read_is_not_a_snapshot() {
        let (_d, mut v) = vault();
        let s = v.begin_read_session("sync", "sim", 1).unwrap();
        for i in 0..497u16 {
            v.record_read_slot(&s, i, &synthetic_payload(i as u32, "x")).unwrap();
        }
        let st = v.finish_read_session(&s, false, "live", "test").unwrap();
        assert_eq!(st.missing, vec![497, 498, 499]);
        assert!(st.snapshot_id.is_none());
        assert!(v.list_snapshots().unwrap().is_empty());
    }

    #[test]
    fn complete_read_seals() {
        let (_d, mut v) = vault();
        let s = v.begin_read_session("sync", "sim", 1).unwrap();
        for i in 0..500u16 {
            v.record_read_slot(&s, i, &synthetic_payload(i as u32, "x")).unwrap();
        }
        let st = v.finish_read_session(&s, false, "live", "test").unwrap();
        let id = st.snapshot_id.unwrap();
        assert!(v.snapshot(&id).unwrap().sealed);
        assert_eq!(v.snapshot_payloads(&id).unwrap()[42], synthetic_payload(42, "x"));
        assert!(v.conn.execute("UPDATE snapshot_slots SET slot=slot WHERE snapshot_id=?1", [&id]).is_err());
        assert!(v.conn.execute("INSERT INTO snapshot_slots VALUES(?1, 0, 'x', NULL)", [&id]).is_err());
    }
}
