// DTOs mirrored from the Rust backend (p6-core / src-tauri). Rust is authoritative.

export const CATEGORIES = ["Bass", "Lead", "Pad", "Keys", "Pluck", "Arp / Sequence", "FX / Texture", "Other"] as const;
export type Category = (typeof CATEGORIES)[number];

export interface ApiError {
  code: string;
  message: string;
  action?: string | null;
  detail?: unknown;
}

export type TransportKind = "Usb" | "Din" | "Simulator";

export interface ConnectionStatus {
  state: "Offline" | "Connected" | "Simulator" | "Disconnected" | "Probing" | "Unresponsive";
  epoch: number;
  kind: TransportKind | null;
  description: string | null;
  simulator_mode: boolean;
  buffer_protected: boolean;
  busy: string | null;
  identity_version: number[] | null;
}

export interface PortList {
  inputs: string[];
  outputs: string[];
  suggested_input: string | null;
  suggested_output: string | null;
}

export interface WriteSessionRow {
  id: string;
  workspace_id: string;
  workspace_revision: number;
  epoch: number;
  device: string;
  simulator: boolean;
  plan_hash: string;
  status: string;
  outcome: string | null;
  error: string | null;
  baseline_snapshot_id: string | null;
  prewrite_snapshot_id: string | null;
  final_snapshot_id: string | null;
  backup_syx_path: string | null;
  backup_manifest_path: string | null;
  parent_session_id: string | null;
  created_ms: number;
  updated_ms: number;
}

export interface WriteStepRow {
  slot: number;
  expected_before_hash: string;
  desired_hash: string;
  state: string;
  attempts: number;
  readback_hash: string | null;
  error: string | null;
}

export interface AppInfo {
  version: string;
  data_dir: string;
  simulator_mode: boolean;
  unfinished_sessions: WriteSessionRow[];
  active_workspace: string | null;
}

export interface Excluded {
  message_index: number;
  byte_offset: number;
  reason: string;
  kind: "Unrelated" | "Unsupported" | "Malformed" | "Truncated";
}

export interface ImportPreview {
  token: string;
  path: string;
  file_name: string;
  file_len: number;
  programs: number;
  edit_buffers: number;
  unique_payloads: number;
  repeated_in_file: number;
  already_in_vault: number;
  repeated_addresses: number;
  excluded: Excluded[];
  complete_user_bank: boolean;
  noncanonical: number;
  error: ApiError | null;
}

export interface ImportSummary {
  source_id: string;
  file_name: string;
  programs: number;
  unique_payloads: number;
  repeated_in_file: number;
  already_in_vault: number;
  excluded: number;
  previously_imported_file: boolean;
  complete_user_bank: boolean;
}

export interface CommitResult {
  token: string;
  summary: ImportSummary | null;
  error: ApiError | null;
}

export interface SourceRow {
  id: string;
  name: string;
  kind: string;
  created_ms: number;
  count: number;
  original_path: string | null;
  file_hash: string | null;
  has_archive: boolean;
}

export interface OccurrenceRow {
  id: string;
  source_id: string;
  source_name: string;
  source_kind: string;
  message_index: number;
  kind: "program" | "edit_buffer";
  address: number | null;
  stored_name: string | null;
  display_name: string;
  vault_label: string | null;
  manual_category: string | null;
  auto_category: string;
  auto_score: number;
  effective_category: string;
  favorite: boolean;
  exact_hash: string;
  ni_hash: string | null;
  dup_exact: number;
  dup_name_only: number;
  params_available: boolean;
  badges: string[];
  noncanonical: boolean;
  format_version: number;
}

export interface ClassificationDetail {
  category: string;
  score: number;
  reasons: string[];
  classifier_version: number;
  params_available: boolean;
}

export interface CellView {
  entry_id: string | null;
  blob_hash: string;
  occurrence_id: string | null;
  name: string;
  category: string;
  manual_category: boolean;
  favorite: boolean;
  source_name: string;
}

export interface SlotView {
  slot: number;
  current: CellView | null;
  new: CellView | null;
  changed: boolean;
}

export interface SnapshotRow {
  id: string;
  kind: string;
  origin: string;
  device: string | null;
  captured_start_ms: number;
  captured_end_ms: number;
  sealed: boolean;
  source_id: string | null;
}

export interface WorkspaceView {
  id: string;
  name: string;
  revision: number;
  baseline: SnapshotRow | null;
  slots: SlotView[];
  changed_count: number;
  empty_count: number;
  can_undo: boolean;
  can_redo: boolean;
  undo_label: string | null;
  redo_label: string | null;
  writable_baseline: boolean;
}

export interface WorkspaceRow {
  id: string;
  name: string;
  baseline_snapshot_id: string | null;
  revision: number;
  archived: boolean;
  updated_ms: number;
}

export type SortKey = "Name" | "Category";

export type WorkspaceOp =
  | { type: "ReplaceFromLibrary"; start: number; occurrence_ids: string[] }
  | { type: "Paste"; start: number; clipboard: string }
  | { type: "CopySlots"; slots: number[]; start: number }
  | { type: "MoveToSlot"; slots: number[]; target: number }
  | { type: "MoveToGap"; slots: number[]; gap: number }
  | { type: "SwapRanges"; a: number; b: number; len: number }
  | { type: "SortSelected"; slots: number[]; key: SortKey }
  | { type: "RevertSelected"; slots: number[] }
  | { type: "ResetToBaseline" };

export type MetaOp =
  | { type: "SetCategory"; occurrence_ids: string[]; category: string | null }
  | { type: "SetFavorite"; occurrence_ids: string[]; favorite: boolean }
  | {
      type: "SetLabels";
      occurrence_ids: string[];
      base: string | null;
      prefix: string;
      suffix: string;
      number_from: number | null;
      clear: boolean;
    };

export interface OpPreview {
  changes: { slot: number; before: string | null; after: string | null }[];
  description: string;
}

export type SlotResolution = "AdoptHardware" | "KeepStaged" | "Agree" | "EmptyStaged" | "Conflict";
export type ConflictChoice = "KeepNew" | "UseSynth";

export interface ReconcileSlot {
  slot: number;
  resolution: SlotResolution;
  old_name: string | null;
  staged_name: string | null;
  live_name: string;
}

export interface ReadSessionState {
  id: string;
  source_id: string;
  received: number;
  missing: number[];
  status: string;
  snapshot_id: string | null;
}

export interface SyncResult {
  read: ReadSessionState;
  workspace_id: string | null;
  created_workspace: boolean;
  rebased: boolean;
  unchanged: boolean;
  reconcile: ReconcileSlot[];
}

export interface PlanStep {
  slot: number;
  expected_before: string;
  desired: string;
  before_name: string;
  desired_name: string;
}

export interface FrozenPlan {
  session_id: string;
  workspace_id: string;
  workspace_revision: number;
  baseline_snapshot_id: string;
  prewrite_snapshot_id: string;
  epoch: number;
  device: string;
  simulator: boolean;
  steps: PlanStep[];
  target: string[];
  backup_syx: string;
  backup_manifest: string;
  backup_hash: string;
  created_ms: number;
}

export interface Review {
  plan: FrozenPlan;
  plan_hash: string;
  per_bank: number[];
  estimated_ms: number;
  transport: string;
}

export type PrepareOutcome =
  | ({ kind: "Ready" } & Review)
  | { kind: "NoChanges"; session_id: string }
  | { kind: "Drift"; session_id: string; live_snapshot_id: string; slots: ReconcileSlot[] };

export interface WriteOutcome {
  session_id: string;
  status: string;
  outcome: string;
  verified: number;
  not_attempted: number;
  failed_or_uncertain: number;
  first_error: string | null;
  final_mismatch_slots: number[];
  final_snapshot_id: string | null;
}

export type Observation = "MatchesDesired" | "MatchesBefore" | "Neither" | "NoReply";

export interface InspectReport {
  session_id: string;
  live_snapshot_id: string | null;
  slots: { slot: number; journal_state: string; observation: Observation; before_name: string; desired_name: string }[];
  matches_desired: number;
  matches_before: number;
  conflicts: number;
  unknown: number;
}

export type RecoveryResult =
  | { kind: "Rebased"; workspace_id: string; revision: number }
  | { kind: "NeedsChoices"; workspace_id: string; live_snapshot_id: string; slots: ReconcileSlot[] }
  | { kind: "RestoreWorkspace"; workspace_id: string; restored_slots: number[]; conflicts: number[] };

export interface ProtectedBuffer {
  id: string;
  blob_hash: string;
  occurrence_id: string | null;
  name: string;
  device: string | null;
  captured_ms: number;
  restored_ms: number | null;
}

export interface AuditionAck {
  status: "queued" | "needs_protection" | "blocked" | "offline";
  id: number;
  message: string | null;
}

export interface Progress {
  op_id: string;
  kind: string;
  epoch: number;
  phase: string;
  done: number;
  total: number;
  slot: number | null;
  message: string | null;
}

export interface OpDone {
  op_id: string;
  kind: string;
  ok: boolean;
  result: unknown;
  error: ApiError | null;
}

export interface DiagEntry {
  at_ms: number;
  direction: string;
  summary: string;
  bytes: number;
  attempt: number;
  elapsed_ms: number;
  result: string;
}
