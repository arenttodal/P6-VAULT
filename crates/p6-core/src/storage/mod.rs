//! SQLite-backed Vault. One serialized owner (the storage service) holds a `Vault`.

pub mod journal;
pub mod library;
pub mod snapshots;
pub mod workspace;

use crate::protocol::payload::{Payload, DECODER_VERSION};
use crate::util::{new_id, now_ms};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::Serialize;
use std::path::{Path, PathBuf};

pub const SCHEMA_VERSION: i64 = 1;

#[derive(Debug, Clone, Serialize, thiserror::Error)]
#[serde(tag = "code", content = "message")]
pub enum VaultError {
    #[error("database error: {0}")]
    Database(String),
    #[error("file error: {0}")]
    Io(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("the workspace changed (revision {current}); refresh and try again")]
    RevisionConflict { current: i64 },
    #[error("{0}")]
    Operation(#[from] crate::workspace::operations::OpError),
    #[error("{0}")]
    Invalid(String),
    #[error("the bank is incomplete: {0} empty slot(s)")]
    IncompleteBank(usize),
    #[error("{0}")]
    Unsupported(String),
}

impl From<rusqlite::Error> for VaultError {
    fn from(e: rusqlite::Error) -> Self {
        VaultError::Database(e.to_string())
    }
}
impl From<std::io::Error> for VaultError {
    fn from(e: std::io::Error) -> Self {
        VaultError::Io(e.to_string())
    }
}

pub type VResult<T> = Result<T, VaultError>;

pub struct Vault {
    pub(crate) conn: Connection,
    root: PathBuf,
    vault_id: String,
}

impl Vault {
    /// Open or create a vault in `root` (the platform app-data directory, or a separate
    /// simulator directory).
    pub fn open(root: &Path) -> VResult<Self> {
        std::fs::create_dir_all(root.join("archives"))?;
        std::fs::create_dir_all(root.join("backups"))?;
        let conn = Connection::open(root.join("vault.sqlite"))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;")?;
        let mut v = Self { conn, root: root.to_path_buf(), vault_id: String::new() };
        v.migrate()?;
        v.vault_id = v.conn.query_row("SELECT value FROM meta WHERE key='vault_id'", [], |r| r.get(0))?;
        Ok(v)
    }

    fn migrate(&mut self) -> VResult<()> {
        self.conn.execute_batch("CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);")?;
        let ver: i64 = self
            .conn
            .query_row("SELECT value FROM meta WHERE key='schema_version'", [], |r| r.get::<_, String>(0))
            .optional()?
            .map(|s| s.parse().unwrap_or(0))
            .unwrap_or(0);
        if ver > SCHEMA_VERSION {
            return Err(VaultError::Unsupported(format!("vault schema {ver} is newer than this app ({SCHEMA_VERSION})")));
        }
        if ver < 1 {
            let tx = self.conn.transaction()?;
            tx.execute_batch(include_str!("schema.sql"))?;
            tx.execute("INSERT INTO meta(key,value) VALUES('schema_version','1')", [])?;
            tx.execute("INSERT OR IGNORE INTO meta(key,value) VALUES('vault_id',?1)", [new_id()])?;
            tx.commit()?;
        }
        Ok(())
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn vault_id(&self) -> &str {
        &self.vault_id
    }
    pub fn archives_dir(&self) -> PathBuf {
        self.root.join("archives")
    }
    pub fn backups_dir(&self) -> PathBuf {
        self.root.join("backups")
    }

    pub fn setting(&self, key: &str) -> VResult<Option<String>> {
        Ok(self.conn.query_row("SELECT value FROM settings WHERE key=?1", [key], |r| r.get(0)).optional()?)
    }
    pub fn set_setting(&self, key: &str, value: &str) -> VResult<()> {
        self.conn.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![key, value])?;
        Ok(())
    }

    pub fn payload(&self, hash: &str) -> VResult<Payload> {
        let b: Vec<u8> = self
            .conn
            .query_row("SELECT payload FROM patch_blobs WHERE hash=?1", [hash], |r| r.get(0))
            .optional()?
            .ok_or_else(|| VaultError::NotFound(format!("patch {}", &hash[..hash.len().min(12)])))?;
        let p = Payload::from_slice(&b).ok_or_else(|| VaultError::Database("stored payload has wrong length".into()))?;
        if p.exact_hash() != hash {
            return Err(VaultError::Database("stored payload hash mismatch".into()));
        }
        Ok(p)
    }

    pub fn occurrence_blob(&self, occurrence_id: &str) -> VResult<String> {
        self.conn
            .query_row("SELECT blob_hash FROM source_occurrences WHERE id=?1", [occurrence_id], |r| r.get(0))
            .optional()?
            .ok_or_else(|| VaultError::NotFound(format!("occurrence {occurrence_id}")))
    }
}

/// Insert a payload blob (dedup by exact hash) and its classification. Returns (hash, was_new).
pub(crate) fn insert_blob(tx: &Transaction, p: &Payload) -> VResult<(String, bool)> {
    let hash = p.exact_hash();
    let layout = p.layout();
    let n = tx.execute(
        "INSERT OR IGNORE INTO patch_blobs(hash,payload,stored_name,ni_hash,layout,format_version,decoder_version) VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![
            hash,
            &p.bytes()[..],
            p.display_name(),
            (layout == crate::protocol::payload::Layout::Chart2016).then(|| p.name_independent_hash()),
            format!("{layout:?}"),
            p.format_version(),
            DECODER_VERSION
        ],
    )?;
    if n > 0 {
        let c = crate::classification::classify(p);
        tx.execute(
            "INSERT OR IGNORE INTO classifications(blob_hash,classifier_version,category,score,evidence,params_available,badges) VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![
                hash,
                c.classifier_version,
                c.category.label(),
                c.score,
                serde_json::to_string(&c.reasons).unwrap(),
                c.parameters_available,
                serde_json::to_string(&c.badges).unwrap()
            ],
        )?;
    }
    Ok((hash, n > 0))
}

pub(crate) fn insert_source(tx: &Transaction, name: &str, kind: &str, original_path: Option<&str>, archive_path: Option<&str>, file_hash: Option<&str>, note: Option<&str>) -> VResult<String> {
    let id = new_id();
    tx.execute(
        "INSERT INTO sources(id,name,kind,original_path,archive_path,file_hash,created_ms,note) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
        params![id, name, kind, original_path, archive_path, file_hash, now_ms(), note],
    )?;
    Ok(id)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn insert_occurrence(tx: &Transaction, source_id: &str, blob: &str, message_index: usize, byte_offset: Option<u64>, kind: &str, address: Option<u16>, frame: Option<&[u8]>, noncanonical: bool) -> VResult<String> {
    let id = new_id();
    tx.execute(
        "INSERT INTO source_occurrences(id,source_id,blob_hash,message_index,byte_offset,kind,orig_address,frame,noncanonical) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![id, source_id, blob, message_index as i64, byte_offset.map(|b| b as i64), kind, address, frame, noncanonical],
    )?;
    Ok(id)
}

#[cfg(test)]
pub(crate) mod testutil {
    use super::*;
    pub fn vault() -> (tempfile::TempDir, Vault) {
        let d = tempfile::tempdir().unwrap();
        let v = Vault::open(d.path()).unwrap();
        (d, v)
    }
}
