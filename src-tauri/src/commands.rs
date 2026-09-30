//! Typed IPC surface. There is deliberately no raw "send MIDI bytes" command.

use crate::errors::{ApiError, ApiResult};
use crate::midi;
use crate::state::{AppState, Connection, ConnectionStatus, PendingImport, SimHandles};
use p6_core::deployment::plan::{self, PrepareOutcome};
use p6_core::deployment::{recovery, sync, writer, Progress};
use p6_core::device::actor::{AuditionOutcome, AuditionRequest, DeviceActor};
use p6_core::device::{Device, DiagEntry, Transport, TransportKind, TransportProfile};
use p6_core::library::export::{self, Expected};
use p6_core::library::import::{preview_import, Excluded};
use p6_core::protocol::payload::Payload;
use p6_core::simulator::SimulatedP6;
use p6_core::slot::UserSlot;
use p6_core::storage::journal::{WriteSessionRow, WriteStepRow};
use p6_core::storage::library::{ClassificationDetail, ImportSummary, OccurrenceRow, ProtectedBuffer, SourceRow};
use p6_core::storage::snapshots::SnapshotRow;
use p6_core::storage::workspace::{MetaOp, OpPreview, WorkspaceOp, WorkspaceRow, WorkspaceView};
use p6_core::workspace::reconcile::ConflictChoice;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, State};

// ---------------------------------------------------------------- app / connection

#[derive(Serialize)]
pub struct AppInfo {
    pub version: String,
    pub data_dir: String,
    pub simulator_mode: bool,
    pub unfinished_sessions: Vec<WriteSessionRow>,
    pub active_workspace: Option<String>,
}

#[tauri::command]
pub fn app_info(state: State<AppState>) -> ApiResult<AppInfo> {
    let v = state.vault();
    let v = v.lock().unwrap();
    Ok(AppInfo {
        version: env!("CARGO_PKG_VERSION").into(),
        data_dir: v.root().to_string_lossy().into(),
        simulator_mode: state.is_simulator(),
        unfinished_sessions: v.unfinished_sessions()?,
        active_workspace: v.active_workspace()?,
    })
}

#[tauri::command]
pub fn set_simulator_mode(app: AppHandle, state: State<AppState>, enabled: bool) -> ApiResult<ConnectionStatus> {
    state.switch_mode(enabled)?;
    let st = state.status();
    let _ = app.emit("connection", &st);
    Ok(st)
}

#[tauri::command]
pub fn list_ports() -> ApiResult<midi::PortList> {
    midi::list_ports().map_err(|e| ApiError::new("MidiError", e))
}

#[tauri::command]
pub fn connection_status(state: State<AppState>) -> ConnectionStatus {
    state.status()
}

fn spawn_actor(app: &AppHandle, device: Device) -> Arc<DeviceActor> {
    let app2 = app.clone();
    Arc::new(DeviceActor::spawn(device, move |o| {
        let payload = match o {
            AuditionOutcome::Sent { id, label } => {
                serde_json::json!({"id": id, "label": label, "status": "sent"})
            }
            AuditionOutcome::Failed { id, label, error } => {
                serde_json::json!({"id": id, "label": label, "status": "failed", "error": error.to_string()})
            }
        };
        let _ = app2.emit("audition", payload);
    }))
}

fn finish_connect(
    app: &AppHandle,
    state: &AppState,
    transport: Box<dyn Transport>,
    kind: TransportKind,
    ports: Option<(String, String)>,
    sim: SimHandles,
) -> ApiResult<ConnectionStatus> {
    if state.busy.lock().unwrap().is_some() {
        return Err(ApiError::new("Busy", "An operation is running."));
    }
    state.disconnect();
    let epoch = state.next_epoch();
    let mut dev = Device::new(transport, TransportProfile::for_kind(kind), epoch);
    let probe = dev.probe()?;
    let description = dev.description();
    let actor = spawn_actor(app, dev);
    *state.conn.lock().unwrap() = Some(Connection { actor, epoch, kind, description, ports, probe, buffer_protected: false, sim });
    let st = state.status();
    let _ = app.emit("connection", &st);
    Ok(st)
}

#[tauri::command]
pub async fn connect(app: AppHandle, state: State<'_, AppState>, input: String, output: String, din: bool) -> ApiResult<ConnectionStatus> {
    if state.is_simulator() {
        return Err(ApiError::new("Invalid", "Turn off Simulator mode to connect real hardware."));
    }
    let t = midi::MidirTransport::open(&input, &output, din).map_err(|e| ApiError::new("MidiError", e))?;
    let kind = if din { TransportKind::Din } else { TransportKind::Usb };
    finish_connect(&app, &state, Box::new(t), kind, Some((input, output)), None)
}

#[tauri::command]
pub async fn connect_simulator(app: AppHandle, state: State<'_, AppState>) -> ApiResult<ConnectionStatus> {
    if !state.is_simulator() {
        return Err(ApiError::new("Invalid", "Enable Simulator mode first. The simulator uses a separate library."));
    }
    let _ = std::fs::create_dir_all(state.sim_dir());
    let sim = SimulatedP6::persistent(state.sim_dir().join("simulated-p6-memory.syx"));
    let handles = Some((sim.state.clone(), sim.control.clone()));
    finish_connect(&app, &state, Box::new(sim), TransportKind::Simulator, None, handles)
}

#[tauri::command]
pub fn disconnect(app: AppHandle, state: State<AppState>) -> ApiResult<ConnectionStatus> {
    if state.busy.lock().unwrap().as_ref().is_some_and(|b| b.1 == "write") {
        return Err(ApiError::new("Busy", "A write is in progress. Stop it first."));
    }
    state.disconnect();
    let st = state.status();
    let _ = app.emit("connection", &st);
    Ok(st)
}

/// Periodic liveness check for hardware ports (CoreMIDI removes unplugged endpoints).
#[tauri::command]
pub fn check_ports(app: AppHandle, state: State<AppState>) -> ConnectionStatus {
    let ports = state.conn.lock().unwrap().as_ref().and_then(|c| c.ports.clone());
    if let Some((i, o)) = ports {
        if state.busy.lock().unwrap().is_none() && !midi::ports_present(&i, &o) {
            state.disconnect();
            let st = state.status();
            let _ = app.emit("connection", &st);
            return ConnectionStatus { state: "Disconnected".into(), ..st };
        }
    }
    state.status()
}

#[tauri::command]
pub fn diagnostics(state: State<AppState>) -> ApiResult<Vec<DiagEntry>> {
    let (actor, _) = state.actor()?;
    Ok(actor.run(|d| d.diagnostics()))
}

// ---------------------------------------------------------------- import / library

#[derive(Serialize)]
pub struct ImportPreviewDto {
    pub token: String,
    pub path: String,
    pub file_name: String,
    pub file_len: usize,
    pub programs: usize,
    pub edit_buffers: usize,
    pub unique_payloads: usize,
    pub repeated_in_file: usize,
    pub already_in_vault: usize,
    pub repeated_addresses: usize,
    pub excluded: Vec<Excluded>,
    pub complete_user_bank: bool,
    pub noncanonical: usize,
    pub error: Option<ApiError>,
}

#[tauri::command]
pub async fn preview_imports(state: State<'_, AppState>, paths: Vec<String>) -> ApiResult<Vec<ImportPreviewDto>> {
    let mut out = Vec::new();
    for path in paths {
        let file_name = PathBuf::from(&path).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| path.clone());
        let empty = |e: ApiError| ImportPreviewDto {
            token: String::new(),
            path: path.clone(),
            file_name: file_name.clone(),
            file_len: 0,
            programs: 0,
            edit_buffers: 0,
            unique_payloads: 0,
            repeated_in_file: 0,
            already_in_vault: 0,
            repeated_addresses: 0,
            excluded: vec![],
            complete_user_bank: false,
            noncanonical: 0,
            error: Some(e),
        };
        let meta = match std::fs::metadata(&path) {
            Ok(m) => m,
            Err(e) => {
                out.push(empty(ApiError::new("Io", e.to_string())));
                continue;
            }
        };
        if meta.len() as usize > p6_core::library::import::MAX_IMPORT_BYTES {
            out.push(empty(ApiError::new("UnsupportedFormat", "File is larger than 32 MiB.")));
            continue;
        }
        let lower = file_name.to_lowercase();
        if lower.ends_with(".p6lib") || lower.ends_with(".p6program") || lower.ends_with(".mid") || lower.ends_with(".midi") {
            out.push(empty(ApiError::new(
                "UnsupportedFormat",
                "This format is not supported. Export the sounds as a standard Prophet-6 .syx file (e.g. from SoundTower or by dumping from the synth) and import that.",
            )));
            continue;
        }
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                out.push(empty(ApiError::new("Io", e.to_string())));
                continue;
            }
        };
        match preview_import(&file_name, &bytes) {
            Ok(p) => {
                let known = state.vault().lock().unwrap().count_known_payloads(&p)?;
                let token = uuid::Uuid::new_v4().to_string();
                let dto = ImportPreviewDto {
                    token: token.clone(),
                    path: path.clone(),
                    file_name: file_name.clone(),
                    file_len: bytes.len(),
                    programs: p.occurrences.iter().filter(|o| o.address.is_some()).count(),
                    edit_buffers: p.occurrences.iter().filter(|o| o.address.is_none()).count(),
                    unique_payloads: p.unique_payloads,
                    repeated_in_file: p.repeated_in_file,
                    already_in_vault: known,
                    repeated_addresses: p.repeated_addresses.len(),
                    excluded: p.excluded.clone(),
                    complete_user_bank: p.complete_user_bank().is_some(),
                    noncanonical: p.occurrences.iter().filter(|o| o.noncanonical).count(),
                    error: None,
                };
                state.previews.lock().unwrap().insert(token, PendingImport { preview: p, bytes, path });
                out.push(dto);
            }
            Err(e) => out.push(empty(ApiError::new("UnsupportedFormat", e.to_string()))),
        }
    }
    Ok(out)
}

#[derive(Serialize)]
pub struct CommitResult {
    pub token: String,
    pub summary: Option<ImportSummary>,
    pub error: Option<ApiError>,
}

#[tauri::command]
pub async fn commit_imports(state: State<'_, AppState>, tokens: Vec<String>) -> ApiResult<Vec<CommitResult>> {
    let mut out = Vec::new();
    for t in tokens {
        let pending = state.previews.lock().unwrap().remove(&t);
        let Some(p) = pending else {
            out.push(CommitResult { token: t, summary: None, error: Some(ApiError::new("NotFound", "Preview expired; import again.")) });
            continue;
        };
        let r = state.vault().lock().unwrap().commit_import(&p.preview, Some(&p.path), &p.bytes);
        out.push(match r {
            Ok(s) => CommitResult { token: t, summary: Some(s), error: None },
            Err(e) => CommitResult { token: t, summary: None, error: Some(e.into()) },
        });
    }
    Ok(out)
}

#[tauri::command]
pub fn discard_imports(state: State<AppState>, tokens: Vec<String>) {
    let mut p = state.previews.lock().unwrap();
    for t in tokens {
        p.remove(&t);
    }
}

#[tauri::command]
pub fn list_sources(state: State<AppState>) -> ApiResult<Vec<SourceRow>> {
    Ok(state.vault().lock().unwrap().list_sources()?)
}

#[tauri::command]
pub async fn list_occurrences(state: State<'_, AppState>) -> ApiResult<Vec<OccurrenceRow>> {
    Ok(state.vault().lock().unwrap().list_occurrences()?)
}

#[tauri::command]
pub fn classification_detail(state: State<AppState>, blob_hash: String) -> ApiResult<ClassificationDetail> {
    Ok(state.vault().lock().unwrap().classification_detail(&blob_hash)?)
}

// ---------------------------------------------------------------- workspaces

#[tauri::command]
pub fn list_workspaces(state: State<AppState>) -> ApiResult<Vec<WorkspaceRow>> {
    Ok(state.vault().lock().unwrap().list_workspaces()?)
}

#[tauri::command]
pub async fn workspace_view(state: State<'_, AppState>, workspace_id: String) -> ApiResult<WorkspaceView> {
    Ok(state.vault().lock().unwrap().workspace_view(&workspace_id)?)
}

#[tauri::command]
pub fn set_active_workspace(state: State<AppState>, workspace_id: String) -> ApiResult<()> {
    Ok(state.vault().lock().unwrap().set_active_workspace(&workspace_id)?)
}

#[tauri::command]
pub fn create_workspace_from_source(state: State<AppState>, source_id: String, name: String) -> ApiResult<String> {
    Ok(state.vault().lock().unwrap().create_workspace_from_source(&source_id, &name)?)
}

#[tauri::command]
pub fn create_empty_workspace(state: State<AppState>, name: String) -> ApiResult<String> {
    Ok(state.vault().lock().unwrap().create_workspace(&name, None, None)?)
}

#[tauri::command]
pub fn list_snapshots(state: State<AppState>) -> ApiResult<Vec<SnapshotRow>> {
    Ok(state.vault().lock().unwrap().list_snapshots()?)
}

#[tauri::command]
pub async fn preview_op(state: State<'_, AppState>, workspace_id: String, op: WorkspaceOp) -> ApiResult<OpPreview> {
    let op = resolve_paste(&state, op)?;
    Ok(state.vault().lock().unwrap().preview_op(&workspace_id, &op)?)
}

fn resolve_paste(state: &AppState, op: WorkspaceOp) -> ApiResult<WorkspaceOp> {
    Ok(match op {
        WorkspaceOp::Paste { start, clipboard } if clipboard.is_empty() => {
            let c = state.clipboard.lock().unwrap().clone().ok_or_else(|| ApiError::new("Invalid", "Nothing has been copied."))?;
            WorkspaceOp::Paste { start, clipboard: c }
        }
        other => other,
    })
}

#[tauri::command]
pub async fn apply_op(state: State<'_, AppState>, workspace_id: String, revision: i64, op: WorkspaceOp) -> ApiResult<i64> {
    let op = resolve_paste(&state, op)?;
    Ok(state.vault().lock().unwrap().apply_op(&workspace_id, revision, &op)?)
}

#[tauri::command]
pub async fn apply_meta(state: State<'_, AppState>, workspace_id: String, revision: i64, op: MetaOp) -> ApiResult<i64> {
    Ok(state.vault().lock().unwrap().apply_meta(&workspace_id, revision, &op)?)
}

#[tauri::command]
pub fn preview_labels(state: State<AppState>, op: MetaOp) -> ApiResult<Vec<(String, String)>> {
    Ok(state.vault().lock().unwrap().preview_labels(&op)?)
}

#[tauri::command]
pub async fn undo(state: State<'_, AppState>, workspace_id: String, revision: i64) -> ApiResult<i64> {
    Ok(state.vault().lock().unwrap().undo(&workspace_id, revision)?)
}

#[tauri::command]
pub async fn redo(state: State<'_, AppState>, workspace_id: String, revision: i64) -> ApiResult<i64> {
    Ok(state.vault().lock().unwrap().redo(&workspace_id, revision)?)
}

#[derive(Deserialize)]
pub struct ClipRef {
    pub blob_hash: String,
    pub occurrence_id: Option<String>,
}

/// Copy: store a versioned Vault clipboard bundle (internal; external text is never a program).
#[tauri::command]
pub fn copy_programs(state: State<AppState>, items: Vec<ClipRef>) -> ApiResult<usize> {
    if items.is_empty() {
        return Err(ApiError::new("Invalid", "Nothing selected."));
    }
    let v = state.vault();
    let v = v.lock().unwrap();
    let n = items.len();
    let bundle = v.make_clipboard(&items.into_iter().map(|i| (i.blob_hash, i.occurrence_id)).collect::<Vec<_>>());
    *state.clipboard.lock().unwrap() = Some(bundle);
    Ok(n)
}

#[tauri::command]
pub fn clipboard_count(state: State<AppState>) -> usize {
    state
        .clipboard
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|c| serde_json::from_str::<serde_json::Value>(c).ok())
        .and_then(|v| v.get("items").and_then(|i| i.as_array()).map(|a| a.len()))
        .unwrap_or(0)
}

// ---------------------------------------------------------------- export

#[tauri::command]
pub async fn export_bank(state: State<'_, AppState>, workspace_id: String, path: String) -> ApiResult<usize> {
    let bank = state.vault().lock().unwrap().staged_payloads(&workspace_id)?;
    let bytes = export::bank_bytes(&bank)?;
    export::write_verified(&PathBuf::from(&path), &bytes, &export::bank_expected(&bank))?;
    Ok(bytes.len())
}

#[derive(Deserialize)]
pub struct SelectionItem {
    pub slot: u16,
    pub blob_hash: String,
}

#[tauri::command]
pub async fn export_selection(state: State<'_, AppState>, items: Vec<SelectionItem>, path: String) -> ApiResult<usize> {
    let v = state.vault();
    let v = v.lock().unwrap();
    let list: Vec<(UserSlot, Payload)> = items
        .iter()
        .map(|i| {
            let s = UserSlot::new(i.slot).ok_or_else(|| ApiError::new("InvalidDestination", format!("{} is not a user slot (000-499)", i.slot)))?;
            Ok((s, v.payload(&i.blob_hash)?))
        })
        .collect::<ApiResult<_>>()?;
    let bytes = export::selection_bytes(&list)?;
    let expected = Expected::Programs(list.iter().map(|(s, p)| (s.address(), p.clone())).collect());
    export::write_verified(&PathBuf::from(&path), &bytes, &expected)?;
    Ok(bytes.len())
}

#[tauri::command]
pub async fn export_source(state: State<'_, AppState>, source_id: String, path: String) -> ApiResult<usize> {
    let (_, archive) = state.vault().lock().unwrap().source_archive(&source_id)?;
    let bytes = std::fs::read(&archive).map_err(|e| ApiError::new("Io", e.to_string()))?;
    let dest = PathBuf::from(&path);
    if dest == archive {
        return Err(ApiError::new("Invalid", "Choose a different destination."));
    }
    export::write_verified(&dest, &bytes, &Expected::Raw)?;
    Ok(bytes.len())
}

#[tauri::command]
pub async fn export_edit_buffer(state: State<'_, AppState>, blob_hash: String, path: String) -> ApiResult<usize> {
    let p = state.vault().lock().unwrap().payload(&blob_hash)?;
    let bytes = export::edit_buffer_bytes(&p);
    export::write_verified(&PathBuf::from(&path), &bytes, &Expected::EditBuffer(p))?;
    Ok(bytes.len())
}

// ---------------------------------------------------------------- long operations

#[derive(Serialize, Clone)]
struct ProgressEvent {
    op_id: String,
    kind: String,
    epoch: u64,
    #[serde(flatten)]
    progress: Progress,
}

#[derive(Serialize, Clone)]
struct DoneEvent {
    op_id: String,
    kind: String,
    ok: bool,
    result: Option<serde_json::Value>,
    error: Option<ApiError>,
}

type OpFn = Box<dyn FnOnce(&mut Device, Arc<Mutex<p6_core::storage::Vault>>, &AtomicBool, &dyn Fn(Progress)) -> ApiResult<serde_json::Value> + Send>;

/// Run an exclusive MIDI transaction in the background. Auditions are blocked and
/// purged for its duration; progress and completion are emitted as events.
fn start_op(app: &AppHandle, state: &AppState, kind: &str, f: OpFn) -> ApiResult<String> {
    let (actor, epoch) = state.actor()?;
    {
        let mut b = state.busy.lock().unwrap();
        if let Some((_, k)) = &*b {
            return Err(ApiError::new("Busy", format!("'{k}' is already running.")));
        }
        let op_id = uuid::Uuid::new_v4().to_string();
        *b = Some((op_id.clone(), kind.to_string()));
        drop(b);
        let cancel = Arc::new(AtomicBool::new(false));
        state.cancels.lock().unwrap().insert(op_id.clone(), cancel.clone());
        actor.set_audition_blocked(true);
        let vault = state.vault();
        let app2 = app.clone();
        let kind2 = kind.to_string();
        let op2 = op_id.clone();
        let _ = app.emit("connection", state.status());
        std::thread::spawn(move || {
            let app3 = app2.clone();
            let op3 = op2.clone();
            let kind3 = kind2.clone();
            let last = Mutex::new(std::time::Instant::now() - Duration::from_secs(1));
            let result = actor.run(move |dev| {
                let progress = move |p: Progress| {
                    let mut l = last.lock().unwrap();
                    // Throttle rendering; always send completion-ish updates.
                    if l.elapsed() > Duration::from_millis(60) || p.slot.is_none() {
                        *l = std::time::Instant::now();
                        let _ = app3.emit("op-progress", ProgressEvent { op_id: op3.clone(), kind: kind3.clone(), epoch, progress: p });
                    }
                };
                f(dev, vault, &cancel, &progress)
            });
            actor.set_audition_blocked(false);
            let st = app2.state::<AppState>();
            *st.busy.lock().unwrap() = None;
            st.cancels.lock().unwrap().remove(&op2);
            let ev = match result {
                Ok(v) => DoneEvent { op_id: op2, kind: kind2, ok: true, result: Some(v), error: None },
                Err(e) => DoneEvent { op_id: op2, kind: kind2, ok: false, result: None, error: Some(e) },
            };
            let _ = app2.emit("op-done", ev);
            let _ = app2.emit("connection", st.status());
        });
        Ok(op_id)
    }
}

use tauri::Manager;

#[tauri::command]
pub fn stop_operation(state: State<AppState>, op_id: String) -> bool {
    if let Some(c) = state.cancels.lock().unwrap().get(&op_id) {
        c.store(true, Ordering::SeqCst);
        return true;
    }
    false
}

#[derive(Serialize)]
struct SyncResult {
    read: p6_core::storage::snapshots::ReadSessionState,
    workspace_id: Option<String>,
    created_workspace: bool,
    rebased: bool,
    unchanged: bool,
    reconcile: Vec<p6_core::storage::workspace::ReconcileSlot>,
}

/// Sync Current P6: read all 500 user slots. A fresh workspace is created from the first
/// complete read; an existing workspace is reconciled (auto-applied only if no choices).
#[tauri::command]
pub fn sync_start(app: AppHandle, state: State<AppState>, retry_session: Option<String>) -> ApiResult<String> {
    start_op(
        &app,
        &state,
        "sync",
        Box::new(move |dev, vault, cancel, progress| {
            let read = sync::read_bank(dev, &vault, retry_session, "sync", "live", cancel, progress)?;
            let mut res = SyncResult { read: read.clone(), workspace_id: None, created_workspace: false, rebased: false, unchanged: false, reconcile: vec![] };
            if let Some(snap) = &read.snapshot_id {
                let mut v = vault.lock().unwrap();
                match v.active_workspace()? {
                    None => {
                        let ws = v.create_workspace("Main", Some(snap), None)?;
                        res.workspace_id = Some(ws);
                        res.created_workspace = true;
                    }
                    Some(ws) => {
                        // Synth unchanged since Current: keep the baseline (and the undo history).
                        if let Some(base) = v.workspace_baseline(&ws)? {
                            let writable = matches!(v.snapshot(&base)?.kind.as_str(), "live" | "post_write" | "prewrite");
                            let same = v.snapshot_cells(&base)?.iter().zip(v.snapshot_cells(snap)?.iter()).all(|(a, b)| a.blob_hash == b.blob_hash);
                            if writable && same {
                                res.workspace_id = Some(ws);
                                res.unchanged = true;
                                return Ok(serde_json::to_value(res).unwrap());
                            }
                        }
                        let slots = v.reconcile_preview(&ws, snap)?;
                        let needs_choice = slots.iter().any(|s| {
                            matches!(
                                s.resolution,
                                p6_core::workspace::reconcile::SlotResolution::Conflict | p6_core::workspace::reconcile::SlotResolution::EmptyStaged
                            )
                        });
                        if !needs_choice {
                            let rev = v.workspace_revision(&ws)?;
                            v.apply_rebase(&ws, rev, snap, &HashMap::new())?;
                            res.rebased = true;
                        }
                        res.reconcile = slots;
                        res.workspace_id = Some(ws);
                    }
                }
            }
            Ok(serde_json::to_value(res).unwrap())
        }),
    )
}

#[tauri::command]
pub async fn apply_rebase(
    state: State<'_, AppState>,
    workspace_id: String,
    revision: i64,
    snapshot_id: String,
    choices: HashMap<usize, ConflictChoice>,
) -> ApiResult<i64> {
    Ok(state.vault().lock().unwrap().apply_rebase(&workspace_id, revision, &snapshot_id, &choices)?)
}

#[tauri::command]
pub fn prepare_review(app: AppHandle, state: State<AppState>, workspace_id: String) -> ApiResult<String> {
    start_op(
        &app,
        &state,
        "prepare",
        Box::new(move |dev, vault, cancel, progress| {
            let out: PrepareOutcome = plan::prepare(dev, &vault, &workspace_id, cancel, progress)?;
            Ok(serde_json::to_value(out).unwrap())
        }),
    )
}

#[tauri::command]
pub fn cancel_review(state: State<AppState>, session_id: String) -> ApiResult<()> {
    let v = state.vault();
    plan::cancel_review(&v, &session_id)?;
    Ok(())
}

/// The user's explicit "Write N programs". The permit is created and consumed inside the
/// MIDI actor; it never crosses IPC. Epoch mismatch (reconnect) invalidates it.
#[tauri::command]
pub fn write_confirmed(app: AppHandle, state: State<AppState>, session_id: String, plan_hash: String) -> ApiResult<String> {
    start_op(
        &app,
        &state,
        "write",
        Box::new(move |dev, vault, cancel, progress| {
            let permit = plan::confirm(&vault, &session_id, &plan_hash, dev.epoch())?;
            let out = writer::execute(dev, &vault, permit, cancel, progress)?;
            Ok(serde_json::to_value(out).unwrap())
        }),
    )
}

#[tauri::command]
pub fn list_write_sessions(state: State<AppState>) -> ApiResult<Vec<WriteSessionRow>> {
    Ok(state.vault().lock().unwrap().list_write_sessions()?)
}

#[tauri::command]
pub fn write_session_steps(state: State<AppState>, session_id: String) -> ApiResult<Vec<WriteStepRow>> {
    Ok(state.vault().lock().unwrap().write_steps(&session_id)?)
}

#[tauri::command]
pub fn inspect_session(app: AppHandle, state: State<AppState>, session_id: String) -> ApiResult<String> {
    let app2 = app.clone();
    start_op(
        &app,
        &state,
        "inspect",
        Box::new(move |dev, vault, cancel, progress| {
            let r = recovery::inspect(dev, &vault, &session_id, cancel, progress)?;
            let json = serde_json::to_value(&r).unwrap();
            app2.state::<AppState>().reports.lock().unwrap().insert(session_id.clone(), r);
            Ok(json)
        }),
    )
}

fn stored_report(state: &AppState, session_id: &str) -> ApiResult<recovery::InspectReport> {
    state
        .reports
        .lock()
        .unwrap()
        .get(session_id)
        .cloned()
        .ok_or_else(|| ApiError::new("Invalid", "Inspect the interrupted write first (with the synth connected)."))
}

#[tauri::command]
pub fn recovery_rebase(
    state: State<AppState>,
    session_id: String,
    choices: HashMap<usize, ConflictChoice>,
    outcome: String,
) -> ApiResult<recovery::RecoveryResult> {
    let report = stored_report(&state, &session_id)?;
    let r = recovery::rebase_after_inspection(&state.vault(), &report, &choices, &outcome)?;
    if matches!(r, recovery::RecoveryResult::Rebased { .. }) {
        state.reports.lock().unwrap().remove(&session_id);
    }
    Ok(r)
}

#[tauri::command]
pub fn recovery_restore(state: State<AppState>, session_id: String) -> ApiResult<recovery::RecoveryResult> {
    let report = stored_report(&state, &session_id)?;
    let r = recovery::restore_affected(&state.vault(), &report)?;
    if let recovery::RecoveryResult::RestoreWorkspace { workspace_id, .. } = &r {
        state.vault().lock().unwrap().set_active_workspace(workspace_id)?;
    }
    state.reports.lock().unwrap().remove(&session_id);
    Ok(r)
}

#[tauri::command]
pub fn stage_backup_restore(state: State<AppState>, backup_snapshot_id: String) -> ApiResult<String> {
    let v = state.vault();
    let active = v.lock().unwrap().active_workspace()?.ok_or_else(|| ApiError::new("Invalid", "Sync with the synth first."))?;
    let base = v.lock().unwrap().workspace_baseline(&active)?.ok_or_else(|| ApiError::new("Invalid", "Sync with the synth first."))?;
    let ws = recovery::stage_backup_restore(&v, &backup_snapshot_id, &base)?;
    v.lock().unwrap().set_active_workspace(&ws)?;
    Ok(ws)
}

// ---------------------------------------------------------------- audition

#[derive(Deserialize)]
#[serde(tag = "kind")]
pub enum AuditionTarget {
    Blob { blob_hash: String, label: String },
}

#[derive(Serialize)]
pub struct AuditionAck {
    pub status: String,
    pub id: u64,
    pub message: Option<String>,
}

#[tauri::command]
pub fn audition(state: State<AppState>, target: AuditionTarget, force: bool) -> ApiResult<AuditionAck> {
    let AuditionTarget::Blob { blob_hash, label } = target;
    let (actor, protected) = {
        let c = state.conn.lock().unwrap();
        match &*c {
            None => return Ok(AuditionAck { status: "offline".into(), id: 0, message: Some("Connect the synth to audition.".into()) }),
            Some(c) => (c.actor.clone(), c.buffer_protected),
        }
    };
    if !protected {
        return Ok(AuditionAck { status: "needs_protection".into(), id: 0, message: None });
    }
    let payload = state.vault().lock().unwrap().payload(&blob_hash)?;
    let id = state.audition_seq.fetch_add(1, Ordering::SeqCst);
    if !actor.audition(AuditionRequest { id, payload, label }, force) {
        return Ok(AuditionAck { status: "blocked".into(), id, message: Some("Auditioning is paused while the bank is being read or written.".into()) });
    }
    Ok(AuditionAck { status: "queued".into(), id, message: None })
}

/// Capture the current edit buffer (unsaved edits) before the first audition.
#[tauri::command]
pub async fn protect_edit_buffer(state: State<'_, AppState>) -> ApiResult<ProtectedBuffer> {
    let (actor, _) = state.actor()?;
    if actor.audition_blocked() {
        return Err(ApiError::new("Busy", "Wait for the current operation."));
    }
    let p = actor.run(|d| d.read_edit_buffer())?;
    let desc = state.status().description.unwrap_or_default();
    let pb = state.vault().lock().unwrap().protect_edit_buffer(&p, &desc)?;
    if let Some(c) = state.conn.lock().unwrap().as_mut() {
        c.buffer_protected = true;
    }
    Ok(pb)
}

#[tauri::command]
pub fn protected_buffers(state: State<AppState>) -> ApiResult<Vec<ProtectedBuffer>> {
    Ok(state.vault().lock().unwrap().protected_buffers()?)
}

/// Restore a protected buffer: first capture the current buffer so it isn't lost, then load (03).
#[tauri::command]
pub async fn restore_protected_buffer(state: State<'_, AppState>, id: String) -> ApiResult<ProtectedBuffer> {
    let (actor, _) = state.actor()?;
    if actor.audition_blocked() {
        return Err(ApiError::new("Busy", "Wait for the current operation."));
    }
    let v = state.vault();
    let target =
        v.lock().unwrap().protected_buffers()?.into_iter().find(|b| b.id == id).ok_or_else(|| ApiError::new("NotFound", "No such protected buffer."))?;
    let payload = v.lock().unwrap().payload(&target.blob_hash)?;
    let current = actor.run(|d| d.read_edit_buffer())?;
    let desc = state.status().description.unwrap_or_default();
    let saved = v.lock().unwrap().protect_edit_buffer(&current, &desc)?;
    actor.run(move |d| d.load_edit_buffer(&payload))?;
    v.lock().unwrap().mark_buffer_restored(&id)?;
    Ok(saved)
}

#[tauri::command]
pub async fn test_note(state: State<'_, AppState>, channel: u8) -> ApiResult<()> {
    let (actor, _) = state.actor()?;
    if actor.audition_blocked() {
        return Err(ApiError::new("Busy", "Unavailable while reading or writing."));
    }
    actor.run(move |d| d.note_on(channel, 60, 80))?;
    let a2 = actor.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(600));
        a2.submit(|d| {
            let _ = d.release_notes();
        });
    });
    Ok(())
}

#[tauri::command]
pub async fn panic(state: State<'_, AppState>, channel: u8) -> ApiResult<()> {
    let (actor, _) = state.actor()?;
    if state.busy.lock().unwrap().as_ref().is_some_and(|b| b.1 == "write") {
        return Err(ApiError::new("Busy", "Unavailable during stored writes."));
    }
    actor.run(move |d| d.panic(channel))?;
    Ok(())
}

// ---------------------------------------------------------------- misc

#[tauri::command]
pub fn get_setting(state: State<AppState>, key: String) -> ApiResult<Option<String>> {
    Ok(state.vault().lock().unwrap().setting(&format!("ui.{key}"))?)
}

#[tauri::command]
pub fn set_setting(state: State<AppState>, key: String, value: String) -> ApiResult<()> {
    Ok(state.vault().lock().unwrap().set_setting(&format!("ui.{key}"), &value)?)
}

#[tauri::command]
pub fn hardware_gate(state: State<AppState>) -> ApiResult<p6_core::storage::journal::HardwareGate> {
    Ok(state.vault().lock().unwrap().hardware_gate()?)
}

#[tauri::command]
pub fn backups_dir(state: State<AppState>) -> String {
    state.vault().lock().unwrap().backups_dir().to_string_lossy().into()
}

// ---------------------------------------------------------------- simulator fault injection

#[derive(Deserialize)]
#[serde(tag = "kind")]
pub enum SimFault {
    /// Someone stores a different program on the synth at this slot (drift).
    ExternalChange {
        slot: u16,
    },
    /// The connection drops after N further stored writes.
    DisconnectAfterWrites {
        writes: u32,
    },
    /// The next N replies are lost.
    DropReplies {
        count: u32,
    },
    Clear,
}

/// Simulator-only test controls. Unavailable (and meaningless) for real hardware.
#[tauri::command]
pub fn simulator_fault(state: State<AppState>, fault: SimFault) -> ApiResult<String> {
    let c = state.conn.lock().unwrap();
    let (st, ctl) = c.as_ref().and_then(|c| c.sim.clone()).ok_or_else(|| ApiError::new("Invalid", "Only available while the simulator is connected."))?;
    drop(c);
    Ok(match fault {
        SimFault::ExternalChange { slot } => {
            if slot >= 500 {
                return Err(ApiError::new("InvalidDestination", "Slot must be 000-499."));
            }
            let n = p6_core::util::now_ms() as u32;
            st.lock().unwrap().programs[slot as usize] = p6_core::protocol::payload::synthetic_payload(n, &format!("Changed on synth {slot:03}"));
            format!("Slot {slot:03} was changed on the simulated synth.")
        }
        SimFault::DisconnectAfterWrites { writes } => {
            ctl.lock().unwrap().disconnect_after_stores = Some(writes);
            format!("The simulated connection will drop after {writes} write(s).")
        }
        SimFault::DropReplies { count } => {
            ctl.lock().unwrap().drop_replies = count;
            format!("The next {count} replies will be lost.")
        }
        SimFault::Clear => {
            let mut c = ctl.lock().unwrap();
            c.disconnect_after_stores = None;
            c.drop_replies = 0;
            c.disconnect_now = false;
            "Faults cleared.".into()
        }
    })
}
