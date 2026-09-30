// Typed IPC layer. Every call goes to Rust; no protocol or bank logic lives here.
import type * as T from "./types";

export interface Backend {
  appInfo(): Promise<T.AppInfo>;
  setSimulatorMode(enabled: boolean): Promise<T.ConnectionStatus>;
  listPorts(): Promise<T.PortList>;
  connectionStatus(): Promise<T.ConnectionStatus>;
  connect(input: string, output: string, din: boolean): Promise<T.ConnectionStatus>;
  connectSimulator(): Promise<T.ConnectionStatus>;
  disconnect(): Promise<T.ConnectionStatus>;
  checkPorts(): Promise<T.ConnectionStatus>;
  diagnostics(): Promise<T.DiagEntry[]>;

  previewImports(paths: string[]): Promise<T.ImportPreview[]>;
  commitImports(tokens: string[]): Promise<T.CommitResult[]>;
  discardImports(tokens: string[]): Promise<void>;
  listSources(): Promise<T.SourceRow[]>;
  listOccurrences(): Promise<T.OccurrenceRow[]>;
  classificationDetail(blobHash: string): Promise<T.ClassificationDetail>;

  listWorkspaces(): Promise<T.WorkspaceRow[]>;
  workspaceView(id: string): Promise<T.WorkspaceView>;
  setActiveWorkspace(id: string): Promise<void>;
  createWorkspaceFromSource(sourceId: string, name: string): Promise<string>;
  createEmptyWorkspace(name: string): Promise<string>;
  listSnapshots(): Promise<T.SnapshotRow[]>;
  previewOp(ws: string, op: T.WorkspaceOp): Promise<T.OpPreview>;
  applyOp(ws: string, revision: number, op: T.WorkspaceOp): Promise<number>;
  applyMeta(ws: string, revision: number, op: T.MetaOp): Promise<number>;
  previewLabels(op: T.MetaOp): Promise<[string, string][]>;
  undo(ws: string, revision: number): Promise<number>;
  redo(ws: string, revision: number): Promise<number>;
  copyPrograms(items: { blob_hash: string; occurrence_id: string | null }[]): Promise<number>;
  clipboardCount(): Promise<number>;

  exportBank(ws: string, path: string): Promise<number>;
  exportSelection(items: { slot: number; blob_hash: string }[], path: string): Promise<number>;
  exportSource(sourceId: string, path: string): Promise<number>;
  exportEditBuffer(blobHash: string, path: string): Promise<number>;

  stopOperation(opId: string): Promise<boolean>;
  syncStart(retrySession: string | null): Promise<string>;
  applyRebase(ws: string, revision: number, snapshotId: string, choices: Record<number, T.ConflictChoice>): Promise<number>;
  prepareReview(ws: string): Promise<string>;
  cancelReview(sessionId: string): Promise<void>;
  writeConfirmed(sessionId: string, planHash: string): Promise<string>;
  listWriteSessions(): Promise<T.WriteSessionRow[]>;
  writeSessionSteps(sessionId: string): Promise<T.WriteStepRow[]>;
  inspectSession(sessionId: string): Promise<string>;
  recoveryRebase(sessionId: string, choices: Record<number, T.ConflictChoice>, outcome: string): Promise<T.RecoveryResult>;
  recoveryRestore(sessionId: string): Promise<T.RecoveryResult>;
  stageBackupRestore(snapshotId: string): Promise<string>;

  audition(blobHash: string, label: string, force: boolean): Promise<T.AuditionAck>;
  protectEditBuffer(): Promise<T.ProtectedBuffer>;
  protectedBuffers(): Promise<T.ProtectedBuffer[]>;
  restoreProtectedBuffer(id: string): Promise<T.ProtectedBuffer>;
  testNote(channel: number): Promise<void>;
  panic(channel: number): Promise<void>;

  getSetting(key: string): Promise<string | null>;
  setSetting(key: string, value: string): Promise<void>;
  backupsDir(): Promise<string>;
  hardwareGate(): Promise<{ passed: boolean; verified_single_slot_sessions: number; required: number }>;

  on<E>(event: string, cb: (payload: E) => void): Promise<() => void>;
  pickFiles(): Promise<string[] | null>;
  saveFile(defaultName: string): Promise<string | null>;
  reveal(path: string): Promise<void>;
  onFileDrop(cb: (paths: string[]) => void): Promise<() => void>;
}

let current: Backend | null = null;
export function setBackend(b: Backend) {
  current = b;
}
export function api(): Backend {
  if (!current) throw new Error("backend not initialised");
  return current;
}
