import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open, save } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import type { Backend } from "./backend";

const i = <R>(cmd: string, args?: Record<string, unknown>) => invoke<R>(cmd, args);

export const tauriBackend: Backend = {
  appInfo: () => i("app_info"),
  setSimulatorMode: (enabled) => i("set_simulator_mode", { enabled }),
  listPorts: () => i("list_ports"),
  connectionStatus: () => i("connection_status"),
  connect: (input, output, din) => i("connect", { input, output, din }),
  connectSimulator: () => i("connect_simulator"),
  disconnect: () => i("disconnect"),
  checkPorts: () => i("check_ports"),
  diagnostics: () => i("diagnostics"),

  previewImports: (paths) => i("preview_imports", { paths }),
  commitImports: (tokens) => i("commit_imports", { tokens }),
  discardImports: (tokens) => i("discard_imports", { tokens }),
  listSources: () => i("list_sources"),
  listOccurrences: () => i("list_occurrences"),
  classificationDetail: (blobHash) => i("classification_detail", { blobHash }),

  listWorkspaces: () => i("list_workspaces"),
  workspaceView: (id) => i("workspace_view", { workspaceId: id }),
  setActiveWorkspace: (id) => i("set_active_workspace", { workspaceId: id }),
  createWorkspaceFromSource: (sourceId, name) => i("create_workspace_from_source", { sourceId, name }),
  createEmptyWorkspace: (name) => i("create_empty_workspace", { name }),
  listSnapshots: () => i("list_snapshots"),
  previewOp: (ws, op) => i("preview_op", { workspaceId: ws, op }),
  applyOp: (ws, revision, op) => i("apply_op", { workspaceId: ws, revision, op }),
  applyMeta: (ws, revision, op) => i("apply_meta", { workspaceId: ws, revision, op }),
  previewLabels: (op) => i("preview_labels", { op }),
  undo: (ws, revision) => i("undo", { workspaceId: ws, revision }),
  redo: (ws, revision) => i("redo", { workspaceId: ws, revision }),
  copyPrograms: (items) => i("copy_programs", { items }),
  clipboardCount: () => i("clipboard_count"),

  exportBank: (ws, path) => i("export_bank", { workspaceId: ws, path }),
  exportSelection: (items, path) => i("export_selection", { items, path }),
  exportSource: (sourceId, path) => i("export_source", { sourceId, path }),
  exportEditBuffer: (blobHash, path) => i("export_edit_buffer", { blobHash, path }),

  stopOperation: (opId) => i("stop_operation", { opId }),
  syncStart: (retrySession) => i("sync_start", { retrySession }),
  applyRebase: (ws, revision, snapshotId, choices) => i("apply_rebase", { workspaceId: ws, revision, snapshotId, choices }),
  prepareReview: (ws) => i("prepare_review", { workspaceId: ws }),
  cancelReview: (sessionId) => i("cancel_review", { sessionId }),
  writeConfirmed: (sessionId, planHash) => i("write_confirmed", { sessionId, planHash }),
  listWriteSessions: () => i("list_write_sessions"),
  writeSessionSteps: (sessionId) => i("write_session_steps", { sessionId }),
  inspectSession: (sessionId) => i("inspect_session", { sessionId }),
  recoveryRebase: (sessionId, choices, outcome) => i("recovery_rebase", { sessionId, choices, outcome }),
  recoveryRestore: (sessionId) => i("recovery_restore", { sessionId }),
  stageBackupRestore: (snapshotId) => i("stage_backup_restore", { backupSnapshotId: snapshotId }),

  audition: (blobHash, label, force) => i("audition", { target: { kind: "Blob", blob_hash: blobHash, label }, force }),
  protectEditBuffer: () => i("protect_edit_buffer"),
  protectedBuffers: () => i("protected_buffers"),
  restoreProtectedBuffer: (id) => i("restore_protected_buffer", { id }),
  testNote: (channel) => i("test_note", { channel }),
  panic: (channel) => i("panic", { channel }),

  getSetting: (key) => i("get_setting", { key }),
  setSetting: (key, value) => i("set_setting", { key, value }),
  backupsDir: () => i("backups_dir"),

  on: async (event, cb) => listen(event, (e) => cb(e.payload as never)),
  pickFiles: async () => {
    const r = await open({ multiple: true, filters: [{ name: "SysEx", extensions: ["syx", "SYX"] }, { name: "All files", extensions: ["*"] }] });
    if (!r) return null;
    return Array.isArray(r) ? r : [r];
  },
  saveFile: async (defaultName) => (await save({ defaultPath: defaultName, filters: [{ name: "SysEx", extensions: ["syx"] }] })) ?? null,
  reveal: (path) => revealItemInDir(path),
  onFileDrop: async (cb) =>
    getCurrentWebview().onDragDropEvent((e) => {
      if (e.payload.type === "drop") cb(e.payload.paths);
    }),
};
