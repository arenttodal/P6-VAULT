//! Application state: the Vault (storage service), the connection (MIDI actor) and
//! long-running operation bookkeeping.

use crate::errors::{ApiError, ApiResult};
use p6_core::device::actor::DeviceActor;
use p6_core::device::{ProbeResult, TransportKind};
use p6_core::library::import::ImportPreview;
use p6_core::storage::Vault;
use serde::Serialize;
use std::collections::HashMap;
use std::fs::File;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

pub struct Connection {
    pub actor: Arc<DeviceActor>,
    pub epoch: u64,
    pub kind: TransportKind,
    pub description: String,
    pub ports: Option<(String, String)>,
    pub probe: ProbeResult,
    /// The edit buffer has been captured (protected) during this connection.
    pub buffer_protected: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConnectionStatus {
    pub state: String,
    pub epoch: u64,
    pub kind: Option<TransportKind>,
    pub description: Option<String>,
    pub simulator_mode: bool,
    pub buffer_protected: bool,
    pub busy: Option<String>,
    pub identity_version: Option<Vec<u8>>,
}

pub struct PendingImport {
    pub preview: ImportPreview,
    pub bytes: Vec<u8>,
    pub path: String,
}

pub struct AppState {
    pub base_dir: PathBuf,
    pub vault: RwLock<Arc<Mutex<Vault>>>,
    pub simulator_mode: AtomicBool,
    pub conn: Mutex<Option<Connection>>,
    pub epoch: AtomicU64,
    pub busy: Mutex<Option<(String, String)>>,
    pub cancels: Mutex<HashMap<String, Arc<AtomicBool>>>,
    pub previews: Mutex<HashMap<String, PendingImport>>,
    pub clipboard: Mutex<Option<String>>,
    pub reports: Mutex<HashMap<String, p6_core::deployment::recovery::InspectReport>>,
    pub audition_seq: AtomicU64,
    pub startup_unfinished: Mutex<Vec<p6_core::storage::journal::WriteSessionRow>>,
    _lock: File,
}

impl AppState {
    pub fn new(base_dir: PathBuf) -> Result<Self, String> {
        std::fs::create_dir_all(&base_dir).map_err(|e| e.to_string())?;
        let lock = File::create(base_dir.join("p6vault.lock")).map_err(|e| e.to_string())?;
        fs2::FileExt::try_lock_exclusive(&lock).map_err(|_| "P6 Vault is already running with this library. Close the other window first.".to_string())?;
        let mut vault = Vault::open(&base_dir).map_err(|e| e.to_string())?;
        let unfinished = vault.startup_recovery_scan().map_err(|e| e.to_string())?;
        Ok(Self {
            base_dir,
            vault: RwLock::new(Arc::new(Mutex::new(vault))),
            simulator_mode: AtomicBool::new(false),
            conn: Mutex::new(None),
            epoch: AtomicU64::new(1),
            busy: Mutex::new(None),
            cancels: Mutex::new(HashMap::new()),
            previews: Mutex::new(HashMap::new()),
            clipboard: Mutex::new(None),
            reports: Mutex::new(HashMap::new()),
            audition_seq: AtomicU64::new(1),
            startup_unfinished: Mutex::new(unfinished),
            _lock: lock,
        })
    }

    pub fn vault(&self) -> Arc<Mutex<Vault>> {
        self.vault.read().unwrap().clone()
    }

    pub fn is_simulator(&self) -> bool {
        self.simulator_mode.load(Ordering::SeqCst)
    }

    pub fn sim_dir(&self) -> PathBuf {
        self.base_dir.join("simulator")
    }

    /// Reopen the Vault for the chosen mode. Disconnects and invalidates everything
    /// connection-bound.
    pub fn switch_mode(&self, simulator: bool) -> ApiResult<()> {
        if self.busy.lock().unwrap().is_some() {
            return Err(ApiError::new("Busy", "Finish the current operation before switching modes."));
        }
        self.disconnect();
        let dir = if simulator { self.sim_dir() } else { self.base_dir.clone() };
        let mut v = Vault::open(&dir)?;
        let unfinished = v.startup_recovery_scan()?;
        *self.vault.write().unwrap() = Arc::new(Mutex::new(v));
        *self.startup_unfinished.lock().unwrap() = unfinished;
        self.simulator_mode.store(simulator, Ordering::SeqCst);
        self.previews.lock().unwrap().clear();
        *self.clipboard.lock().unwrap() = None;
        Ok(())
    }

    pub fn next_epoch(&self) -> u64 {
        self.epoch.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn disconnect(&self) {
        let old = self.conn.lock().unwrap().take();
        if let Some(c) = old {
            c.actor.set_audition_blocked(true);
            drop(c);
        }
        // Invalidate epoch-bound permits/pending work.
        self.next_epoch();
        self.reports.lock().unwrap().clear();
    }

    pub fn status(&self) -> ConnectionStatus {
        let c = self.conn.lock().unwrap();
        let busy = self.busy.lock().unwrap().as_ref().map(|b| b.1.clone());
        match &*c {
            Some(c) => ConnectionStatus {
                state: if c.kind == TransportKind::Simulator { "Simulator".into() } else { "Connected".into() },
                epoch: c.epoch,
                kind: Some(c.kind),
                description: Some(c.description.clone()),
                simulator_mode: self.is_simulator(),
                buffer_protected: c.buffer_protected,
                busy,
                identity_version: c.probe.identity_version.clone(),
            },
            None => ConnectionStatus {
                state: "Offline".into(),
                epoch: self.epoch.load(Ordering::SeqCst),
                kind: None,
                description: None,
                simulator_mode: self.is_simulator(),
                buffer_protected: false,
                busy,
                identity_version: None,
            },
        }
    }

    pub fn actor(&self) -> ApiResult<(Arc<DeviceActor>, u64)> {
        let c = self.conn.lock().unwrap();
        let c = c.as_ref().ok_or_else(|| ApiError::new("Offline", "No synth is connected."))?;
        Ok((c.actor.clone(), c.epoch))
    }
}
