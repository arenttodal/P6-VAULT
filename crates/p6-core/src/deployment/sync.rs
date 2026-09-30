//! Sequential full-bank reads into the Vault. Each slot is persisted as it arrives.
//! Only a complete 500-slot read becomes a sealed snapshot.

use super::*;
use crate::device::Device;
use crate::slot::UserSlot;
use crate::storage::snapshots::ReadSessionState;
use crate::storage::Vault;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// Read `slots` (all 500 when None) into read session `session` (created if None).
/// Stops issuing new requests on cancel; keeps what was received.
pub fn read_bank(
    dev: &mut Device,
    vault: &Mutex<Vault>,
    session: Option<String>,
    purpose: &str,
    kind: &str,
    cancel: &AtomicBool,
    progress: &dyn Fn(Progress),
) -> DResult<ReadSessionState> {
    let session = match session {
        Some(s) => s,
        None => vault.lock().unwrap().begin_read_session(purpose, &dev.description(), dev.epoch())?,
    };
    let todo: Vec<u16> = vault.lock().unwrap().read_session_state(&session)?.missing;
    let total = 500;
    let mut done = total - todo.len();
    let mut cancelled = false;
    for slot in todo {
        if cancel.load(Ordering::Relaxed) {
            cancelled = true;
            break;
        }
        progress(Progress::new(purpose, done, total, Some(slot)));
        match dev.read_program(UserSlot::new(slot).unwrap().address(), Some(cancel)) {
            Ok(p) => {
                vault.lock().unwrap().record_read_slot(&session, slot, &p)?;
                done += 1;
            }
            // Unanswered slots stay missing and can be retried.
            Err(DeviceError::DeviceUnresponsive { .. }) => {}
            Err(DeviceError::Cancelled) => {
                cancelled = true;
                break;
            }
            Err(e) => {
                // Disconnect/overflow: keep what we have, report partial.
                vault.lock().unwrap().finish_read_session(&session, false, kind, purpose)?;
                return Err(e.into());
            }
        }
    }
    progress(Progress::new(purpose, done, total, None));
    Ok(vault.lock().unwrap().finish_read_session(&session, cancelled, kind, purpose)?)
}
