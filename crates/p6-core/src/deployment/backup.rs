//! Standard 500-program .syx backup plus JSON manifest, flushed and re-verified.

use super::*;
use crate::library::export::{bank_bytes, bank_expected, write_verified};
use crate::protocol::payload::Payload;
use crate::storage::Vault;
use serde::Serialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize)]
pub struct BackupInfo {
    pub syx_path: PathBuf,
    pub manifest_path: PathBuf,
    pub file_hash: String,
}

#[derive(Serialize)]
struct Manifest<'a> {
    app: &'a str,
    app_version: &'a str,
    schema_version: i64,
    protocol: &'a str,
    session_id: &'a str,
    snapshot_id: &'a str,
    capture_start_ms: i64,
    capture_end_ms: i64,
    device: Option<String>,
    transport_profile: serde_json::Value,
    simulator: bool,
    file_hash: String,
    slots: Vec<String>,
}

pub fn write_backup(vault: &Vault, session_id: &str, snapshot_id: &str, transport_profile: serde_json::Value, simulator: bool) -> DResult<BackupInfo> {
    let snap = vault.snapshot(snapshot_id)?;
    if !snap.sealed {
        return Err(DeployError::BackupFailed("snapshot is not complete".into()));
    }
    let payloads: Vec<Payload> = vault.snapshot_payloads(snapshot_id)?;
    let bank: Vec<Option<Payload>> = payloads.iter().cloned().map(Some).collect();
    let bytes = bank_bytes(&bank).map_err(|e| DeployError::BackupFailed(e.to_string()))?;
    let base = format!("P6-backup-{}-{}", crate::util::file_timestamp(snap.captured_end_ms), &session_id[..8]);
    let dir = vault.backups_dir();
    std::fs::create_dir_all(&dir).map_err(|e| DeployError::BackupFailed(e.to_string()))?;
    let syx = dir.join(format!("{base}.syx"));
    let man = dir.join(format!("{base}.json"));
    if syx.exists() || man.exists() {
        return Err(DeployError::BackupFailed("backup file name collision".into()));
    }
    write_verified(&syx, &bytes, &bank_expected(&bank)).map_err(|e| DeployError::BackupFailed(e.to_string()))?;
    let file_hash = crate::library::import::file_hash(&bytes);
    let manifest = Manifest {
        app: "P6 Vault",
        app_version: env!("CARGO_PKG_VERSION"),
        schema_version: crate::storage::SCHEMA_VERSION,
        protocol: "Prophet-6 SysEx program dump, command 02, 1178 bytes/program",
        session_id,
        snapshot_id,
        capture_start_ms: snap.captured_start_ms,
        capture_end_ms: snap.captured_end_ms,
        device: snap.device.clone(),
        transport_profile,
        simulator,
        file_hash: file_hash.clone(),
        slots: payloads.iter().map(|p| p.exact_hash()).collect(),
    };
    let json = serde_json::to_vec_pretty(&manifest).unwrap();
    write_verified(&man, &json, &crate::library::export::Expected::Raw).map_err(|e| DeployError::BackupFailed(e.to_string()))?;
    Ok(BackupInfo { syx_path: syx, manifest_path: man, file_hash })
}
