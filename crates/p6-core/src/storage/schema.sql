CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);

CREATE TABLE sources (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  kind TEXT NOT NULL CHECK (kind IN ('file','live_snapshot','partial_read','edit_buffer_capture','backup','post_write','derived')),
  original_path TEXT,
  archive_path TEXT,
  file_hash TEXT,
  created_ms INTEGER NOT NULL,
  note TEXT
);

CREATE TABLE patch_blobs (
  hash TEXT PRIMARY KEY,
  payload BLOB NOT NULL CHECK (length(payload) = 1024),
  stored_name TEXT,
  ni_hash TEXT,
  layout TEXT NOT NULL,
  format_version INTEGER NOT NULL,
  decoder_version INTEGER NOT NULL
);
CREATE INDEX blobs_ni ON patch_blobs(ni_hash);

CREATE TABLE source_occurrences (
  id TEXT PRIMARY KEY,
  source_id TEXT NOT NULL REFERENCES sources(id),
  blob_hash TEXT NOT NULL REFERENCES patch_blobs(hash),
  message_index INTEGER NOT NULL,
  byte_offset INTEGER,
  kind TEXT NOT NULL CHECK (kind IN ('program','edit_buffer')),
  orig_address INTEGER CHECK (orig_address IS NULL OR orig_address BETWEEN 0 AND 999),
  frame BLOB,
  noncanonical INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX occ_source ON source_occurrences(source_id, message_index);
CREATE INDEX occ_blob ON source_occurrences(blob_hash);

CREATE TABLE patch_annotations (
  occurrence_id TEXT PRIMARY KEY REFERENCES source_occurrences(id),
  vault_label TEXT,
  manual_category TEXT,
  favorite INTEGER NOT NULL DEFAULT 0,
  updated_ms INTEGER NOT NULL
);

CREATE TABLE classifications (
  blob_hash TEXT NOT NULL REFERENCES patch_blobs(hash),
  classifier_version INTEGER NOT NULL,
  category TEXT NOT NULL,
  score REAL NOT NULL,
  evidence TEXT NOT NULL,
  params_available INTEGER NOT NULL,
  badges TEXT NOT NULL,
  PRIMARY KEY (blob_hash, classifier_version)
);

CREATE TABLE derived_versions (
  derived_hash TEXT PRIMARY KEY REFERENCES patch_blobs(hash),
  parent_hash TEXT NOT NULL REFERENCES patch_blobs(hash),
  occurrence_id TEXT REFERENCES source_occurrences(id),
  reason TEXT NOT NULL,
  evidence TEXT,
  created_ms INTEGER NOT NULL,
  accepted INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE read_sessions (
  id TEXT PRIMARY KEY,
  purpose TEXT NOT NULL,
  status TEXT NOT NULL CHECK (status IN ('running','complete','partial','cancelled','failed')),
  device TEXT,
  epoch INTEGER,
  started_ms INTEGER NOT NULL,
  finished_ms INTEGER,
  received INTEGER NOT NULL DEFAULT 0,
  missing TEXT,
  source_id TEXT REFERENCES sources(id),
  snapshot_id TEXT
);

CREATE TABLE snapshots (
  id TEXT PRIMARY KEY,
  kind TEXT NOT NULL CHECK (kind IN ('live','imported','post_write','prewrite')),
  origin TEXT NOT NULL,
  device TEXT,
  captured_start_ms INTEGER NOT NULL,
  captured_end_ms INTEGER NOT NULL,
  sealed INTEGER NOT NULL DEFAULT 0,
  source_id TEXT REFERENCES sources(id)
);

CREATE TABLE snapshot_slots (
  snapshot_id TEXT NOT NULL REFERENCES snapshots(id),
  slot INTEGER NOT NULL CHECK (slot BETWEEN 0 AND 499),
  blob_hash TEXT NOT NULL REFERENCES patch_blobs(hash),
  occurrence_id TEXT REFERENCES source_occurrences(id),
  PRIMARY KEY (snapshot_id, slot)
);

CREATE TRIGGER snapshot_slots_immutable BEFORE UPDATE ON snapshot_slots
BEGIN SELECT RAISE(ABORT, 'snapshot slots are immutable'); END;
CREATE TRIGGER snapshot_sealed_no_insert BEFORE INSERT ON snapshot_slots
WHEN (SELECT sealed FROM snapshots WHERE id = NEW.snapshot_id) = 1
BEGIN SELECT RAISE(ABORT, 'snapshot is sealed'); END;
CREATE TRIGGER blobs_immutable BEFORE UPDATE OF payload ON patch_blobs
BEGIN SELECT RAISE(ABORT, 'patch blobs are immutable'); END;

CREATE TABLE workspaces (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  baseline_snapshot_id TEXT REFERENCES snapshots(id),
  revision INTEGER NOT NULL DEFAULT 0,
  branch INTEGER NOT NULL DEFAULT 0,
  history_pos INTEGER NOT NULL DEFAULT 0,
  ui_context TEXT,
  archived INTEGER NOT NULL DEFAULT 0,
  created_ms INTEGER NOT NULL,
  updated_ms INTEGER NOT NULL
);

CREATE TABLE workspace_slots (
  workspace_id TEXT NOT NULL REFERENCES workspaces(id),
  slot INTEGER NOT NULL CHECK (slot BETWEEN 0 AND 499),
  entry_id TEXT,
  blob_hash TEXT REFERENCES patch_blobs(hash),
  occurrence_id TEXT REFERENCES source_occurrences(id),
  PRIMARY KEY (workspace_id, slot),
  CHECK ((entry_id IS NULL) = (blob_hash IS NULL))
);

CREATE TABLE workspace_history (
  workspace_id TEXT NOT NULL REFERENCES workspaces(id),
  branch INTEGER NOT NULL,
  seq INTEGER NOT NULL,
  op_type TEXT NOT NULL,
  description TEXT NOT NULL,
  slots_before TEXT NOT NULL,
  slots_after TEXT NOT NULL,
  meta_before TEXT NOT NULL,
  meta_after TEXT NOT NULL,
  created_ms INTEGER NOT NULL,
  PRIMARY KEY (workspace_id, branch, seq)
);

CREATE TABLE workspace_checkpoints (
  id TEXT PRIMARY KEY,
  workspace_id TEXT NOT NULL REFERENCES workspaces(id),
  reason TEXT NOT NULL,
  baseline_snapshot_id TEXT,
  slots TEXT NOT NULL,
  created_ms INTEGER NOT NULL
);

CREATE TABLE write_sessions (
  id TEXT PRIMARY KEY,
  workspace_id TEXT NOT NULL REFERENCES workspaces(id),
  workspace_revision INTEGER NOT NULL,
  epoch INTEGER NOT NULL,
  device TEXT NOT NULL,
  simulator INTEGER NOT NULL,
  plan_json TEXT NOT NULL,
  plan_hash TEXT NOT NULL,
  baseline_snapshot_id TEXT REFERENCES snapshots(id),
  prewrite_snapshot_id TEXT REFERENCES snapshots(id),
  final_snapshot_id TEXT REFERENCES snapshots(id),
  backup_syx_path TEXT,
  backup_manifest_path TEXT,
  backup_hash TEXT,
  parent_session_id TEXT,
  status TEXT NOT NULL CHECK (status IN ('Preparing','Ready','Writing','Reconciling','Completed','CancelledBeforeWrite','Interrupted','NeedsRecovery','Closed')),
  outcome TEXT,
  error TEXT,
  created_ms INTEGER NOT NULL,
  updated_ms INTEGER NOT NULL
);

CREATE TABLE write_steps (
  session_id TEXT NOT NULL REFERENCES write_sessions(id),
  slot INTEGER NOT NULL CHECK (slot BETWEEN 0 AND 499),
  ord INTEGER NOT NULL,
  expected_before_hash TEXT NOT NULL REFERENCES patch_blobs(hash),
  desired_hash TEXT NOT NULL REFERENCES patch_blobs(hash),
  state TEXT NOT NULL CHECK (state IN ('Planned','SendIntent','SentUnverified','Verified','Failed','Uncertain')),
  attempts INTEGER NOT NULL DEFAULT 0,
  readback_hash TEXT,
  error TEXT,
  updated_ms INTEGER NOT NULL,
  PRIMARY KEY (session_id, slot)
);

CREATE TABLE protected_buffers (
  id TEXT PRIMARY KEY,
  blob_hash TEXT NOT NULL REFERENCES patch_blobs(hash),
  occurrence_id TEXT REFERENCES source_occurrences(id),
  device TEXT,
  captured_ms INTEGER NOT NULL,
  restored_ms INTEGER
);

CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
