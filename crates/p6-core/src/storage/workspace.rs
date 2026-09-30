//! Workspaces: the editable 500-slot New bank, its baseline, undo history and rebasing.

use super::library::Annotation;
use super::snapshots::SnapshotRow;
use super::*;
use crate::classification::Category;
use crate::workspace::operations::{self as ops, Bank, Entry, OpError, BANK_LEN};
use crate::workspace::reconcile::{resolve, ConflictChoice, SlotResolution};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize)]
pub struct CellView {
    pub entry_id: Option<String>,
    pub blob_hash: String,
    pub occurrence_id: Option<String>,
    pub name: String,
    pub category: String,
    pub manual_category: bool,
    pub favorite: bool,
    pub source_name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SlotView {
    pub slot: usize,
    pub current: Option<CellView>,
    pub new: Option<CellView>,
    pub changed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceView {
    pub id: String,
    pub name: String,
    pub revision: i64,
    pub baseline: Option<SnapshotRow>,
    pub slots: Vec<SlotView>,
    pub changed_count: usize,
    pub empty_count: usize,
    pub can_undo: bool,
    pub can_redo: bool,
    pub undo_label: Option<String>,
    pub redo_label: Option<String>,
    pub writable_baseline: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceRow {
    pub id: String,
    pub name: String,
    pub baseline_snapshot_id: Option<String>,
    pub revision: i64,
    pub archived: bool,
    pub updated_ms: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SortKey {
    Name,
    Category,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WorkspaceOp {
    /// Library -> New, replacing start..start+k (occurrences in library display order).
    ReplaceFromLibrary {
        start: usize,
        occurrence_ids: Vec<String>,
    },
    Paste {
        start: usize,
        clipboard: String,
    },
    /// Copy New slots (physical order) to start, replacing.
    CopySlots {
        slots: Vec<usize>,
        start: usize,
    },
    MoveToSlot {
        slots: Vec<usize>,
        target: usize,
    },
    MoveToGap {
        slots: Vec<usize>,
        gap: usize,
    },
    SwapRanges {
        a: usize,
        b: usize,
        len: usize,
    },
    SortSelected {
        slots: Vec<usize>,
        key: SortKey,
    },
    RevertSelected {
        slots: Vec<usize>,
    },
    ResetToBaseline,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum MetaOp {
    SetCategory {
        occurrence_ids: Vec<String>,
        category: Option<String>,
    },
    SetFavorite {
        occurrence_ids: Vec<String>,
        favorite: bool,
    },
    /// Bulk Vault labels: base (existing name if None) + prefix/suffix + optional numbering.
    SetLabels {
        occurrence_ids: Vec<String>,
        base: Option<String>,
        prefix: String,
        suffix: String,
        number_from: Option<u32>,
        clear: bool,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct PreviewChange {
    pub slot: usize,
    pub before: Option<String>,
    pub after: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OpPreview {
    pub changes: Vec<PreviewChange>,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ClipboardBundle {
    app: String,
    version: u32,
    vault_id: String,
    items: Vec<ClipItem>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ClipItem {
    blob_hash: String,
    occurrence_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReconcileSlot {
    pub slot: usize,
    pub resolution: SlotResolution,
    pub old_name: Option<String>,
    pub staged_name: Option<String>,
    pub live_name: String,
}

type Changes = Vec<(usize, Option<Entry>)>;
/// A cell as (blob hash, occurrence id); None = empty.
pub type CellRef = Option<(String, Option<String>)>;

struct CellInfo {
    name: String,
    category: String,
    manual: bool,
    favorite: bool,
    source: String,
}

impl Vault {
    fn ws_meta(&self, ws: &str) -> VResult<(String, Option<String>, i64, i64, i64)> {
        self.conn
            .query_row("SELECT name, baseline_snapshot_id, revision, branch, history_pos FROM workspaces WHERE id=?1", [ws], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })
            .optional()?
            .ok_or_else(|| VaultError::NotFound(format!("workspace {ws}")))
    }

    pub fn workspace_revision(&self, ws: &str) -> VResult<i64> {
        Ok(self.ws_meta(ws)?.2)
    }

    pub fn workspace_baseline(&self, ws: &str) -> VResult<Option<String>> {
        Ok(self.ws_meta(ws)?.1)
    }

    pub fn list_workspaces(&self) -> VResult<Vec<WorkspaceRow>> {
        let mut st = self.conn.prepare("SELECT id,name,baseline_snapshot_id,revision,archived,updated_ms FROM workspaces ORDER BY updated_ms DESC")?;
        let rows = st
            .query_map([], |r| {
                Ok(WorkspaceRow {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    baseline_snapshot_id: r.get(2)?,
                    revision: r.get(3)?,
                    archived: r.get(4)?,
                    updated_ms: r.get(5)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }

    pub fn active_workspace(&self) -> VResult<Option<String>> {
        let id = self.setting("active_workspace")?;
        match id {
            Some(id) if self.ws_meta(&id).is_ok() => Ok(Some(id)),
            _ => Ok(None),
        }
    }

    pub fn set_active_workspace(&self, ws: &str) -> VResult<()> {
        self.ws_meta(ws)?;
        self.set_setting("active_workspace", ws)
    }

    /// New workspace. `baseline` = complete sealed snapshot (New cloned from it), or
    /// `cells` for an unfinished offline workspace (explicit empty cells allowed).
    pub fn create_workspace(
        &mut self,
        name: &str,
        baseline: Option<&str>,
        cells: Option<Vec<CellRef>>,
    ) -> VResult<String> {
        let cells: Vec<CellRef> = match (baseline, cells) {
            (_, Some(c)) => c,
            (Some(b), None) => self
                .snapshot_cells(b)?
                .into_iter()
                .map(|c| Some((c.blob_hash, c.occurrence_id)))
                .collect(),
            (None, None) => vec![None; BANK_LEN],
        };
        if cells.len() != BANK_LEN {
            return Err(VaultError::Invalid(
                "a workspace needs exactly 500 cells".into(),
            ));
        }
        if let Some(b) = baseline {
            if !self.snapshot(b)?.sealed {
                return Err(VaultError::Invalid(
                    "baseline snapshot is not sealed".into(),
                ));
            }
        }
        let id = new_id();
        let now = now_ms();
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO workspaces(id,name,baseline_snapshot_id,revision,branch,history_pos,created_ms,updated_ms) VALUES(?1,?2,?3,0,0,0,?4,?4)",
            params![id, name, baseline, now],
        )?;
        for (i, c) in cells.into_iter().enumerate() {
            let (e, b, o) = match c {
                Some((b, o)) => (Some(new_id()), Some(b), o),
                None => (None, None, None),
            };
            tx.execute("INSERT INTO workspace_slots(workspace_id,slot,entry_id,blob_hash,occurrence_id) VALUES(?1,?2,?3,?4,?5)", params![id, i as i64, e, b, o])?;
        }
        tx.execute(
            "INSERT INTO settings(key,value) VALUES('active_workspace',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [&id],
        )?;
        tx.commit()?;
        Ok(id)
    }

    /// Workspace from an imported source: complete bank -> Imported baseline; otherwise
    /// an unfinished workspace with explicit empty cells at missing destinations.
    pub fn create_workspace_from_source(&mut self, source_id: &str, name: &str) -> VResult<String> {
        match self.snapshot_from_source(source_id, "imported") {
            Ok(snap) => self.create_workspace(name, Some(&snap), None),
            Err(VaultError::IncompleteBank(_)) => {
                let mut cells: Vec<CellRef> = vec![None; BANK_LEN];
                for (occ, addr, hash) in self.source_programs(source_id)? {
                    if let Some(a) = addr.filter(|a| *a < 500) {
                        if cells[a as usize].is_some() {
                            return Err(VaultError::Invalid(format!("source has more than one program for slot {a:03}; build the bank explicitly")));
                        }
                        cells[a as usize] = Some((hash, Some(occ)));
                    }
                }
                self.create_workspace(name, None, Some(cells))
            }
            Err(e) => Err(e),
        }
    }

    pub fn load_bank(&self, ws: &str) -> VResult<Bank> {
        let mut st = self.conn.prepare_cached("SELECT entry_id, blob_hash, occurrence_id FROM workspace_slots WHERE workspace_id=?1 ORDER BY slot")?;
        let bank: Bank = st
            .query_map([ws], |r| {
                let e: Option<String> = r.get(0)?;
                let b: Option<String> = r.get(1)?;
                Ok(match (e, b) {
                    (Some(entry_id), Some(blob_hash)) => Some(Entry {
                        entry_id,
                        blob_hash,
                        occurrence_id: r.get(2)?,
                    }),
                    _ => None,
                })
            })?
            .collect::<Result<_, _>>()?;
        if bank.len() != BANK_LEN {
            return Err(VaultError::Database(format!(
                "workspace has {} slots",
                bank.len()
            )));
        }
        Ok(bank)
    }

    fn baseline_bank(&self, ws: &str) -> VResult<Option<Bank>> {
        match self.ws_meta(ws)?.1 {
            None => Ok(None),
            Some(snap) => Ok(Some(
                self.snapshot_cells(&snap)?
                    .into_iter()
                    .map(|c| {
                        Some(Entry {
                            entry_id: new_id(),
                            blob_hash: c.blob_hash,
                            occurrence_id: c.occurrence_id,
                        })
                    })
                    .collect(),
            )),
        }
    }

    /// Staged payloads for export/deployment. None cells are empty.
    pub fn staged_payloads(&self, ws: &str) -> VResult<Vec<Option<Payload>>> {
        self.load_bank(ws)?
            .iter()
            .map(|c| c.as_ref().map(|e| self.payload(&e.blob_hash)).transpose())
            .collect()
    }

    fn cell_infos(&self, ws: &str, snap: Option<&str>) -> VResult<HashMap<String, CellInfo>> {
        let mut st = self.conn.prepare(
            "SELECT o.id, COALESCE(NULLIF(TRIM(a.vault_label),''), b.stored_name), COALESCE(a.manual_category, c.category, 'Other'),
                    a.manual_category IS NOT NULL, COALESCE(a.favorite,0), s.name, o.orig_address, o.message_index
             FROM source_occurrences o JOIN sources s ON s.id=o.source_id JOIN patch_blobs b ON b.hash=o.blob_hash
             LEFT JOIN patch_annotations a ON a.occurrence_id=o.id
             LEFT JOIN classifications c ON c.blob_hash=b.hash AND c.classifier_version=1
             WHERE o.id IN (SELECT occurrence_id FROM workspace_slots WHERE workspace_id=?1
                            UNION SELECT occurrence_id FROM snapshot_slots WHERE snapshot_id=?2)",
        )?;
        let map = st
            .query_map(params![ws, snap], |r| {
                let src: String = r.get(5)?;
                let addr: Option<i64> = r.get(6)?;
                let fallback = match addr {
                    Some(a) => format!("{src} #{a:03}"),
                    None => format!("{src} (edit buffer #{})", r.get::<_, i64>(7)?),
                };
                Ok((
                    r.get::<_, String>(0)?,
                    CellInfo {
                        name: r.get::<_, Option<String>>(1)?.unwrap_or(fallback),
                        category: r.get(2)?,
                        manual: r.get(3)?,
                        favorite: r.get::<_, i64>(4)? != 0,
                        source: src,
                    },
                ))
            })?
            .collect::<Result<_, _>>()?;
        Ok(map)
    }

    fn blob_name(&self, hash: &str) -> String {
        self.conn
            .query_row(
                "SELECT stored_name FROM patch_blobs WHERE hash=?1",
                [hash],
                |r| r.get::<_, Option<String>>(0),
            )
            .ok()
            .flatten()
            .unwrap_or_else(|| format!("#{}", &hash[..8]))
    }

    fn cell_view(
        &self,
        infos: &HashMap<String, CellInfo>,
        entry_id: Option<String>,
        blob: &str,
        occ: Option<&String>,
    ) -> CellView {
        let info = occ.and_then(|o| infos.get(o));
        CellView {
            entry_id,
            blob_hash: blob.to_string(),
            occurrence_id: occ.cloned(),
            name: info
                .map(|i| i.name.clone())
                .unwrap_or_else(|| self.blob_name(blob)),
            category: info
                .map(|i| i.category.clone())
                .unwrap_or_else(|| "Other".into()),
            manual_category: info.is_some_and(|i| i.manual),
            favorite: info.is_some_and(|i| i.favorite),
            source_name: info.map(|i| i.source.clone()).unwrap_or_default(),
        }
    }

    pub fn workspace_view(&self, ws: &str) -> VResult<WorkspaceView> {
        let (name, snap, revision, branch, pos) = self.ws_meta(ws)?;
        let bank = self.load_bank(ws)?;
        let base_cells = match &snap {
            Some(s) => Some(self.snapshot_cells(s)?),
            None => None,
        };
        let infos = self.cell_infos(ws, snap.as_deref())?;
        let mut slots = Vec::with_capacity(BANK_LEN);
        for (i, cell) in bank.iter().enumerate() {
            let cur = base_cells.as_ref().map(|b| &b[i]);
            let changed = match (cur, cell) {
                (Some(c), Some(e)) => c.blob_hash != e.blob_hash,
                (None, _) => false,
                (Some(_), None) => true,
            };
            slots.push(SlotView {
                slot: i,
                current: cur
                    .map(|c| self.cell_view(&infos, None, &c.blob_hash, c.occurrence_id.as_ref())),
                new: cell.as_ref().map(|e| {
                    self.cell_view(
                        &infos,
                        Some(e.entry_id.clone()),
                        &e.blob_hash,
                        e.occurrence_id.as_ref(),
                    )
                }),
                changed,
            });
        }
        let label = |seq: i64| -> Option<String> {
            self.conn
                .query_row("SELECT description FROM workspace_history WHERE workspace_id=?1 AND branch=?2 AND seq=?3", params![ws, branch, seq], |r| r.get(0))
                .optional()
                .ok()
                .flatten()
        };
        let undo_label = if pos > 0 { label(pos) } else { None };
        let redo_label = label(pos + 1);
        let baseline = snap.as_deref().map(|s| self.snapshot(s)).transpose()?;
        Ok(WorkspaceView {
            id: ws.to_string(),
            name,
            revision,
            writable_baseline: baseline
                .as_ref()
                .is_some_and(|b| matches!(b.kind.as_str(), "live" | "post_write" | "prewrite")),
            baseline,
            changed_count: slots.iter().filter(|s| s.changed).count(),
            empty_count: bank.iter().filter(|c| c.is_none()).count(),
            slots,
            can_undo: undo_label.is_some(),
            can_redo: redo_label.is_some(),
            undo_label,
            redo_label,
        })
    }

    fn check_rev(&self, ws: &str, expected: i64) -> VResult<()> {
        let cur = self.workspace_revision(ws)?;
        if cur != expected {
            return Err(VaultError::RevisionConflict { current: cur });
        }
        Ok(())
    }

    fn entries_from_occurrences(&self, ids: &[String]) -> VResult<Vec<Entry>> {
        ids.iter()
            .map(|o| {
                Ok(Entry {
                    entry_id: new_id(),
                    blob_hash: self.occurrence_blob(o)?,
                    occurrence_id: Some(o.clone()),
                })
            })
            .collect()
    }

    /// Versioned clipboard bundle referencing immutable patches in this Vault.
    pub fn make_clipboard(&self, items: &[(String, Option<String>)]) -> String {
        serde_json::to_string(&ClipboardBundle {
            app: "p6-vault".into(),
            version: 1,
            vault_id: self.vault_id.clone(),
            items: items
                .iter()
                .map(|(b, o)| ClipItem {
                    blob_hash: b.clone(),
                    occurrence_id: o.clone(),
                })
                .collect(),
        })
        .unwrap()
    }

    fn entries_from_clipboard(&self, text: &str) -> VResult<Vec<Entry>> {
        let b: ClipboardBundle = serde_json::from_str(text).map_err(|_| {
            VaultError::Invalid("clipboard does not contain P6 Vault programs".into())
        })?;
        if b.app != "p6-vault" || b.version != 1 {
            return Err(VaultError::Invalid("unsupported clipboard format".into()));
        }
        if b.vault_id != self.vault_id {
            return Err(VaultError::Invalid(
                "clipboard came from a different Vault".into(),
            ));
        }
        b.items
            .into_iter()
            .map(|i| {
                self.payload(&i.blob_hash)?;
                if let Some(o) = &i.occurrence_id {
                    if self.occurrence_blob(o)? != i.blob_hash {
                        return Err(VaultError::Invalid(
                            "clipboard reference does not match".into(),
                        ));
                    }
                }
                Ok(Entry {
                    entry_id: new_id(),
                    blob_hash: i.blob_hash,
                    occurrence_id: i.occurrence_id,
                })
            })
            .collect()
    }

    fn compute(&self, ws: &str, op: &WorkspaceOp) -> VResult<(Bank, Bank, String)> {
        let bank = self.load_bank(ws)?;
        let (after, desc) = match op {
            WorkspaceOp::ReplaceFromLibrary {
                start,
                occurrence_ids,
            } => {
                let items = self.entries_from_occurrences(occurrence_ids)?;
                (
                    ops::replace_at(&bank, *start, items)?,
                    format!("Place {} program(s) at {:03}", occurrence_ids.len(), start),
                )
            }
            WorkspaceOp::Paste { start, clipboard } => {
                let items = self.entries_from_clipboard(clipboard)?;
                let n = items.len();
                (
                    ops::replace_at(&bank, *start, items)?,
                    format!("Paste {n} program(s) at {start:03}"),
                )
            }
            WorkspaceOp::CopySlots { slots, start } => {
                let mut s = slots.clone();
                s.sort_unstable();
                s.dedup();
                let items: Vec<Entry> = s
                    .iter()
                    .map(|&i| {
                        bank.get(i)
                            .and_then(|c| c.clone())
                            .map(|e| Entry {
                                entry_id: new_id(),
                                ..e
                            })
                            .ok_or(VaultError::Operation(OpError::EmptySlot(i)))
                    })
                    .collect::<VResult<_>>()?;
                (
                    ops::replace_at(&bank, *start, items)?,
                    format!("Copy {} slot(s) to {start:03}", s.len()),
                )
            }
            WorkspaceOp::MoveToSlot { slots, target } => (
                ops::move_to_slot(&bank, slots, *target)?,
                format!("Move {} slot(s) to {target:03}", slots.len()),
            ),
            WorkspaceOp::MoveToGap { slots, gap } => (
                ops::move_to_gap(&bank, slots, *gap)?,
                format!("Move {} slot(s)", slots.len()),
            ),
            WorkspaceOp::SwapRanges { a, b, len } => (
                ops::swap_ranges(&bank, *a, *b, *len)?,
                format!(
                    "Swap {a:03}-{:03} with {b:03}-{:03}",
                    a + len - 1,
                    b + len - 1
                ),
            ),
            WorkspaceOp::SortSelected { slots, key } => {
                let infos = self.cell_infos(ws, None)?;
                let k = |c: &Option<Entry>| -> (usize, String) {
                    let info = c
                        .as_ref()
                        .and_then(|e| e.occurrence_id.as_ref())
                        .and_then(|o| infos.get(o));
                    let name = info
                        .map(|i| i.name.to_lowercase())
                        .unwrap_or_else(|| "\u{10FFFF}".into());
                    match key {
                        SortKey::Name => (0, name),
                        SortKey::Category => (
                            info.and_then(|i| Category::from_label(&i.category))
                                .map(|c| c.order())
                                .unwrap_or(99),
                            name,
                        ),
                    }
                };
                (
                    ops::sort_selected(&bank, slots, k)?,
                    format!("Sort {} slot(s) by {key:?}", slots.len()),
                )
            }
            WorkspaceOp::RevertSelected { slots } => {
                let base = self.baseline_bank(ws)?.ok_or_else(|| {
                    VaultError::Invalid("this workspace has no complete Current baseline".into())
                })?;
                // Keep identical cells untouched so reverting an unchanged slot is a no-op.
                let mut out = ops::revert_selected(&bank, &base, slots)?;
                for (i, c) in out.iter_mut().enumerate() {
                    if bank[i].as_ref().map(|e| &e.blob_hash) == c.as_ref().map(|e| &e.blob_hash) {
                        *c = bank[i].clone();
                    }
                }
                (out, format!("Revert {} slot(s) to Current", slots.len()))
            }
            WorkspaceOp::ResetToBaseline => {
                let base = self.baseline_bank(ws)?.ok_or_else(|| {
                    VaultError::Invalid("this workspace has no complete Current baseline".into())
                })?;
                let out: Bank = base
                    .into_iter()
                    .enumerate()
                    .map(|(i, c)| {
                        if bank[i].as_ref().map(|e| &e.blob_hash)
                            == c.as_ref().map(|e| &e.blob_hash)
                        {
                            bank[i].clone()
                        } else {
                            c
                        }
                    })
                    .collect();
                (out, "Reset New to Current".to_string())
            }
        };
        Ok((bank, after, desc))
    }

    pub fn preview_op(&self, ws: &str, op: &WorkspaceOp) -> VResult<OpPreview> {
        let (before, after, description) = self.compute(ws, op)?;
        let infos = self.cell_infos(ws, None)?;
        let mut extra: Vec<String> = after
            .iter()
            .flatten()
            .filter_map(|e| e.occurrence_id.clone())
            .filter(|o| !infos.contains_key(o))
            .collect();
        extra.dedup();
        let extra_names: HashMap<String, String> = self
            .occurrences_by_id(&extra)?
            .into_iter()
            .map(|r| (r.id, r.display_name))
            .collect();
        let name = |c: &Option<Entry>| {
            c.as_ref().map(|e| {
                e.occurrence_id
                    .as_ref()
                    .and_then(|o| {
                        infos
                            .get(o)
                            .map(|i| i.name.clone())
                            .or_else(|| extra_names.get(o).cloned())
                    })
                    .unwrap_or_else(|| self.blob_name(&e.blob_hash))
            })
        };
        let changes = ops::changed_slots(&before, &after)
            .into_iter()
            .map(|s| PreviewChange {
                slot: s,
                before: name(&before[s]),
                after: name(&after[s]),
            })
            .collect();
        Ok(OpPreview {
            changes,
            description,
        })
    }

    /// Apply an offline operation atomically with history. No-ops add no history.
    pub fn apply_op(&mut self, ws: &str, expected_rev: i64, op: &WorkspaceOp) -> VResult<i64> {
        self.check_rev(ws, expected_rev)?;
        self.ensure_not_frozen(ws)?;
        let (before, after, desc) = self.compute(ws, op)?;
        let changed = ops::changed_slots(&before, &after);
        if changed.is_empty() {
            return Ok(expected_rev);
        }
        let b: Changes = changed.iter().map(|&s| (s, before[s].clone())).collect();
        let a: Changes = changed.iter().map(|&s| (s, after[s].clone())).collect();
        let op_type = serde_json::to_value(op)
            .ok()
            .and_then(|v| v.get("type").and_then(|t| t.as_str()).map(String::from))
            .unwrap_or_default();
        self.commit_change(ws, &op_type, &desc, b, a, vec![], vec![])
    }

    pub fn apply_meta(&mut self, ws: &str, expected_rev: i64, op: &MetaOp) -> VResult<i64> {
        self.check_rev(ws, expected_rev)?;
        let (ids, desc) = match op {
            MetaOp::SetCategory {
                occurrence_ids,
                category,
            } => (
                occurrence_ids,
                match category {
                    Some(c) => format!("Set category {c} on {}", occurrence_ids.len()),
                    None => format!("Clear category override on {}", occurrence_ids.len()),
                },
            ),
            MetaOp::SetFavorite {
                occurrence_ids,
                favorite,
            } => (
                occurrence_ids,
                format!(
                    "{} {}",
                    if *favorite { "Favorite" } else { "Unfavorite" },
                    occurrence_ids.len()
                ),
            ),
            MetaOp::SetLabels { occurrence_ids, .. } => (
                occurrence_ids,
                format!("Edit Vault labels on {}", occurrence_ids.len()),
            ),
        };
        let mut uniq = ids.clone();
        uniq.dedup();
        if uniq.is_empty() {
            return Err(VaultError::Operation(OpError::EmptySelection));
        }
        let before = self.annotations(&uniq)?;
        let rows: HashMap<String, super::library::OccurrenceRow> = self
            .occurrences_by_id(&uniq)?
            .into_iter()
            .map(|r| (r.id.clone(), r))
            .collect();
        let mut after = before.clone();
        for (i, a) in after.iter_mut().enumerate() {
            match op {
                MetaOp::SetCategory { category, .. } => {
                    if let Some(c) = category {
                        if Category::from_label(c).is_none() {
                            return Err(VaultError::Invalid(format!("unknown category '{c}'")));
                        }
                    }
                    a.manual_category = category.clone();
                }
                MetaOp::SetFavorite { favorite, .. } => a.favorite = *favorite,
                MetaOp::SetLabels {
                    base,
                    prefix,
                    suffix,
                    number_from,
                    clear,
                    ..
                } => {
                    if *clear {
                        a.vault_label = None;
                    } else {
                        let r = &rows[&a.occurrence_id];
                        let b = base.clone().unwrap_or_else(|| {
                            r.stored_name
                                .clone()
                                .unwrap_or_else(|| r.display_name.clone())
                        });
                        let num = number_from
                            .map(|n| format!(" {}", n as usize + i))
                            .unwrap_or_default();
                        a.vault_label = Some(format!("{prefix}{b}{suffix}{num}"));
                    }
                }
            }
        }
        if before == after {
            return Ok(expected_rev);
        }
        self.commit_change(ws, "Meta", &desc, vec![], vec![], before, after)
    }

    pub fn preview_labels(&self, op: &MetaOp) -> VResult<Vec<(String, String)>> {
        let MetaOp::SetLabels {
            occurrence_ids,
            base,
            prefix,
            suffix,
            number_from,
            clear,
        } = op
        else {
            return Err(VaultError::Invalid("not a label operation".into()));
        };
        let rows = self.occurrences_by_id(occurrence_ids)?;
        Ok(rows
            .into_iter()
            .enumerate()
            .map(|(i, r)| {
                let new = if *clear {
                    r.stored_name.clone().unwrap_or_default()
                } else {
                    let b = base.clone().unwrap_or_else(|| {
                        r.stored_name
                            .clone()
                            .unwrap_or_else(|| r.display_name.clone())
                    });
                    format!(
                        "{prefix}{b}{suffix}{}",
                        number_from
                            .map(|n| format!(" {}", n as usize + i))
                            .unwrap_or_default()
                    )
                };
                (r.display_name, new)
            })
            .collect())
    }

    #[allow(clippy::too_many_arguments)]
    fn commit_change(
        &mut self,
        ws: &str,
        op_type: &str,
        desc: &str,
        sb: Changes,
        sa: Changes,
        mb: Vec<Annotation>,
        ma: Vec<Annotation>,
    ) -> VResult<i64> {
        let (_, _, rev, branch, pos) = self.ws_meta(ws)?;
        let tx = self.conn.transaction()?;
        write_slots(&tx, ws, &sa)?;
        Vault::write_annotations(&tx, &ma)?;
        tx.execute(
            "DELETE FROM workspace_history WHERE workspace_id=?1 AND branch=?2 AND seq>?3",
            params![ws, branch, pos],
        )?;
        tx.execute(
            "INSERT INTO workspace_history(workspace_id,branch,seq,op_type,description,slots_before,slots_after,meta_before,meta_after,created_ms) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![
                ws,
                branch,
                pos + 1,
                op_type,
                desc,
                serde_json::to_string(&sb).unwrap(),
                serde_json::to_string(&sa).unwrap(),
                serde_json::to_string(&mb).unwrap(),
                serde_json::to_string(&ma).unwrap(),
                now_ms()
            ],
        )?;
        tx.execute(
            "UPDATE workspaces SET revision=revision+1, history_pos=?2, updated_ms=?3 WHERE id=?1",
            params![ws, pos + 1, now_ms()],
        )?;
        tx.commit()?;
        Ok(rev + 1)
    }

    fn step_history(&mut self, ws: &str, expected_rev: i64, undo: bool) -> VResult<i64> {
        self.check_rev(ws, expected_rev)?;
        let (_, _, rev, branch, pos) = self.ws_meta(ws)?;
        let seq = if undo { pos } else { pos + 1 };
        if seq < 1 {
            return Err(VaultError::Invalid("nothing to undo".into()));
        }
        let row: Option<(String, String, String, String)> = self
            .conn
            .query_row(
                "SELECT slots_before, slots_after, meta_before, meta_after FROM workspace_history WHERE workspace_id=?1 AND branch=?2 AND seq=?3",
                params![ws, branch, seq],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        let (sb, sa, mb, ma) = row.ok_or_else(|| {
            VaultError::Invalid(
                if undo {
                    "nothing to undo"
                } else {
                    "nothing to redo"
                }
                .into(),
            )
        })?;
        let (slots, meta) = if undo { (sb, mb) } else { (sa, ma) };
        let slots: Changes =
            serde_json::from_str(&slots).map_err(|e| VaultError::Database(e.to_string()))?;
        let meta: Vec<Annotation> =
            serde_json::from_str(&meta).map_err(|e| VaultError::Database(e.to_string()))?;
        if !slots.is_empty() {
            self.ensure_not_frozen(ws)?;
        }
        let tx = self.conn.transaction()?;
        write_slots(&tx, ws, &slots)?;
        Vault::write_annotations(&tx, &meta)?;
        let new_pos = if undo { pos - 1 } else { pos + 1 };
        tx.execute(
            "UPDATE workspaces SET revision=revision+1, history_pos=?2, updated_ms=?3 WHERE id=?1",
            params![ws, new_pos, now_ms()],
        )?;
        tx.commit()?;
        Ok(rev + 1)
    }

    pub fn undo(&mut self, ws: &str, expected_rev: i64) -> VResult<i64> {
        self.step_history(ws, expected_rev, true)
    }
    pub fn redo(&mut self, ws: &str, expected_rev: i64) -> VResult<i64> {
        self.step_history(ws, expected_rev, false)
    }

    /// Workspace edits are frozen while a write session is being prepared/reviewed/written.
    fn ensure_not_frozen(&self, ws: &str) -> VResult<()> {
        let frozen: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM write_sessions WHERE workspace_id=?1 AND status IN ('Preparing','Ready','Writing','Reconciling'))",
            [ws],
            |r| r.get(0),
        )?;
        if frozen {
            return Err(VaultError::Invalid(
                "New is frozen while a write is being reviewed or performed".into(),
            ));
        }
        Ok(())
    }

    pub fn checkpoint(&mut self, ws: &str, reason: &str) -> VResult<String> {
        let bank = self.load_bank(ws)?;
        let base = self.workspace_baseline(ws)?;
        let id = new_id();
        self.conn.execute(
            "INSERT INTO workspace_checkpoints(id,workspace_id,reason,baseline_snapshot_id,slots,created_ms) VALUES(?1,?2,?3,?4,?5,?6)",
            params![id, ws, reason, base, serde_json::to_string(&bank).unwrap(), now_ms()],
        )?;
        Ok(id)
    }

    /// Three-way comparison against a fresh complete live snapshot.
    pub fn reconcile_preview(&self, ws: &str, live_snapshot: &str) -> VResult<Vec<ReconcileSlot>> {
        let bank = self.load_bank(ws)?;
        // Only a hardware-derived baseline is a trustworthy "O". With an imported or missing
        // baseline, every slot where New differs from the synth is an explicit choice.
        let base = match self.workspace_baseline(ws)? {
            Some(b)
                if matches!(
                    self.snapshot(&b)?.kind.as_str(),
                    "live" | "post_write" | "prewrite"
                ) =>
            {
                Some(self.snapshot_cells(&b)?)
            }
            _ => None,
        };
        let live = self.snapshot_cells(live_snapshot)?;
        let infos = self.cell_infos(
            ws,
            base.as_ref().and(self.workspace_baseline(ws)?).as_deref(),
        )?;
        let live_rows: HashMap<String, String> = self
            .occurrences_by_id(
                &live
                    .iter()
                    .filter_map(|c| c.occurrence_id.clone())
                    .collect::<Vec<_>>(),
            )?
            .into_iter()
            .map(|r| (r.id, r.display_name))
            .collect();
        let mut out = Vec::new();
        for i in 0..BANK_LEN {
            let o = base.as_ref().map(|b| b[i].blob_hash.as_str());
            let s = bank[i].as_ref().map(|e| e.blob_hash.as_str());
            let h = live[i].blob_hash.as_str();
            let r = resolve(o, s, h);
            let trivially_same = s == Some(h) && o.is_none_or(|o| o == h);
            if trivially_same {
                continue;
            }
            let nm = |occ: Option<&String>, blob: &str| {
                occ.and_then(|x| infos.get(x))
                    .map(|i| i.name.clone())
                    .unwrap_or_else(|| self.blob_name(blob))
            };
            out.push(ReconcileSlot {
                slot: i,
                resolution: r,
                old_name: base
                    .as_ref()
                    .map(|b| nm(b[i].occurrence_id.as_ref(), &b[i].blob_hash)),
                staged_name: bank[i]
                    .as_ref()
                    .map(|e| nm(e.occurrence_id.as_ref(), &e.blob_hash)),
                live_name: live[i]
                    .occurrence_id
                    .as_ref()
                    .and_then(|o| live_rows.get(o).cloned())
                    .unwrap_or_else(|| self.blob_name(h)),
            });
        }
        Ok(out)
    }

    /// Rebase New onto a fresh live snapshot. Every Conflict/EmptyStaged slot needs a choice.
    /// Checkpoints first, then seals the live snapshot as the new baseline and starts a new
    /// history branch.
    pub fn apply_rebase(
        &mut self,
        ws: &str,
        expected_rev: i64,
        live_snapshot: &str,
        choices: &HashMap<usize, ConflictChoice>,
    ) -> VResult<i64> {
        self.check_rev(ws, expected_rev)?;
        self.ensure_not_frozen(ws)?;
        let preview = self.reconcile_preview(ws, live_snapshot)?;
        let live = self.snapshot_cells(live_snapshot)?;
        let mut bank = self.load_bank(ws)?;
        for r in &preview {
            let use_live = match r.resolution {
                SlotResolution::AdoptHardware => true,
                SlotResolution::KeepStaged | SlotResolution::Agree => false,
                SlotResolution::Conflict | SlotResolution::EmptyStaged => {
                    match choices.get(&r.slot) {
                        Some(ConflictChoice::UseSynth) => true,
                        Some(ConflictChoice::KeepNew) => false,
                        None => {
                            return Err(VaultError::Invalid(format!(
                                "slot {:03} needs a choice: Keep New or Use Synth",
                                r.slot
                            )))
                        }
                    }
                }
            };
            if use_live {
                bank[r.slot] = Some(Entry {
                    entry_id: new_id(),
                    blob_hash: live[r.slot].blob_hash.clone(),
                    occurrence_id: live[r.slot].occurrence_id.clone(),
                });
            }
        }
        self.checkpoint(ws, "before rebase onto hardware snapshot")?;
        let tx = self.conn.transaction()?;
        let all: Changes = bank.iter().cloned().enumerate().collect();
        write_slots(&tx, ws, &all)?;
        tx.execute(
            "UPDATE workspaces SET baseline_snapshot_id=?2, revision=revision+1, branch=branch+1, history_pos=0, updated_ms=?3 WHERE id=?1",
            params![ws, live_snapshot, now_ms()],
        )?;
        tx.commit()?;
        self.workspace_revision(ws)
    }

    /// Advance the baseline after a fully verified deployment (New unchanged).
    pub(crate) fn advance_baseline(&mut self, ws: &str, snapshot: &str) -> VResult<()> {
        self.checkpoint(ws, "before baseline advance after verified write")?;
        self.conn.execute(
            "UPDATE workspaces SET baseline_snapshot_id=?2, revision=revision+1, branch=branch+1, history_pos=0, updated_ms=?3 WHERE id=?1",
            params![ws, snapshot, now_ms()],
        )?;
        Ok(())
    }

    /// Create a separate recoverable workspace whose New is `cells` and baseline is `baseline`.
    pub fn create_workspace_with_cells(
        &mut self,
        name: &str,
        baseline: Option<&str>,
        cells: Vec<CellRef>,
    ) -> VResult<String> {
        self.create_workspace(name, baseline, Some(cells))
    }
}

fn write_slots(tx: &Transaction, ws: &str, changes: &Changes) -> VResult<()> {
    let mut st = tx.prepare_cached("UPDATE workspace_slots SET entry_id=?3, blob_hash=?4, occurrence_id=?5 WHERE workspace_id=?1 AND slot=?2")?;
    for (s, c) in changes {
        if *s >= BANK_LEN {
            return Err(VaultError::Invalid(format!("slot {s} out of range")));
        }
        let (e, b, o) = match c {
            Some(e) => (
                Some(&e.entry_id),
                Some(&e.blob_hash),
                e.occurrence_id.as_ref(),
            ),
            None => (None, None, None),
        };
        st.execute(params![ws, *s as i64, e, b, o])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::testutil::vault;
    use super::*;
    use crate::library::export::bank_bytes;
    use crate::library::import::preview_import;
    use crate::protocol::payload::synthetic_payload;

    fn setup() -> (tempfile::TempDir, Vault, String, String) {
        let (d, mut v) = vault();
        let bank: Vec<Option<Payload>> = (0..500)
            .map(|i| Some(synthetic_payload(i, &format!("Prog {i:03}"))))
            .collect();
        let bytes = bank_bytes(&bank).unwrap();
        let p = preview_import("bank.syx", &bytes).unwrap();
        let s = v.commit_import(&p, None, &bytes).unwrap();
        let ws = v
            .create_workspace_from_source(&s.source_id, "Main")
            .unwrap();
        (d, v, ws, s.source_id)
    }

    #[test]
    fn offline_flow_with_undo() {
        let (_d, mut v, ws, _) = setup();
        let view = v.workspace_view(&ws).unwrap();
        assert_eq!(view.changed_count, 0);
        assert_eq!(view.baseline.as_ref().unwrap().kind, "imported");
        assert!(!view.writable_baseline);
        let e_before = v.load_bank(&ws).unwrap();

        let rev = v
            .apply_op(
                &ws,
                0,
                &WorkspaceOp::MoveToSlot {
                    slots: vec![1, 3],
                    target: 4,
                },
            )
            .unwrap();
        assert_eq!(rev, 1);
        let after = v.load_bank(&ws).unwrap();
        assert_eq!(after[4], e_before[1]); // entry ids stable across moves
        assert_eq!(v.workspace_view(&ws).unwrap().changed_count, 5);
        // stale revision rejected
        assert!(matches!(
            v.apply_op(&ws, 0, &WorkspaceOp::ResetToBaseline),
            Err(VaultError::RevisionConflict { current: 1 })
        ));
        // no-op adds no history
        assert_eq!(
            v.apply_op(
                &ws,
                1,
                &WorkspaceOp::MoveToGap {
                    slots: vec![10, 11],
                    gap: 11
                }
            )
            .unwrap(),
            1
        );
        let rev = v.undo(&ws, 1).unwrap();
        assert_eq!(v.load_bank(&ws).unwrap(), e_before);
        let rev = v.redo(&ws, rev).unwrap();
        assert_eq!(v.load_bank(&ws).unwrap(), after);
        let rev = v
            .apply_op(
                &ws,
                rev,
                &WorkspaceOp::RevertSelected {
                    slots: (0..500).collect(),
                },
            )
            .unwrap();
        assert_eq!(v.workspace_view(&ws).unwrap().changed_count, 0);
        assert!(v.workspace_view(&ws).unwrap().can_undo);
        let _ = rev;
    }

    #[test]
    fn hundred_undos_persist_across_reopen() {
        let (d, mut v, ws, _) = setup();
        let orig = v.load_bank(&ws).unwrap();
        let mut rev = 0;
        for i in 0..120 {
            rev = v
                .apply_op(
                    &ws,
                    rev,
                    &WorkspaceOp::SwapRanges {
                        a: 0,
                        b: 10 + (i % 50),
                        len: 2,
                    },
                )
                .unwrap();
        }
        drop(v);
        let mut v = Vault::open(d.path()).unwrap();
        for _ in 0..120 {
            rev = v.undo(&ws, rev).unwrap();
        }
        assert_eq!(v.load_bank(&ws).unwrap(), orig);
        assert!(v.undo(&ws, rev).is_err());
    }

    #[test]
    fn library_replace_and_clipboard() {
        let (_d, mut v, ws, _) = setup();
        let occ = v.list_occurrences().unwrap();
        let ids: Vec<String> = occ.iter().take(20).map(|o| o.id.clone()).collect();
        assert!(matches!(
            v.apply_op(
                &ws,
                0,
                &WorkspaceOp::ReplaceFromLibrary {
                    start: 481,
                    occurrence_ids: ids.clone()
                }
            ),
            Err(VaultError::Operation(OpError::SelectionOverflow {
                highest_valid_start: 480,
                ..
            }))
        ));
        assert_eq!(v.workspace_revision(&ws).unwrap(), 0);
        let rev = v
            .apply_op(
                &ws,
                0,
                &WorkspaceOp::ReplaceFromLibrary {
                    start: 480,
                    occurrence_ids: ids,
                },
            )
            .unwrap();
        assert_eq!(v.workspace_view(&ws).unwrap().changed_count, 20);
        let clip = v.make_clipboard(&[
            (occ[5].exact_hash.clone(), Some(occ[5].id.clone())),
            (occ[5].exact_hash.clone(), Some(occ[5].id.clone())),
        ]);
        let rev = v
            .apply_op(
                &ws,
                rev,
                &WorkspaceOp::Paste {
                    start: 0,
                    clipboard: clip,
                },
            )
            .unwrap();
        let b = v.load_bank(&ws).unwrap();
        assert_eq!(
            b[0].as_ref().unwrap().blob_hash,
            b[1].as_ref().unwrap().blob_hash
        );
        assert_ne!(
            b[0].as_ref().unwrap().entry_id,
            b[1].as_ref().unwrap().entry_id
        );
        assert!(v
            .apply_op(
                &ws,
                rev,
                &WorkspaceOp::Paste {
                    start: 0,
                    clipboard: "garbage".into()
                }
            )
            .is_err());
    }

    #[test]
    fn meta_ops_undo_and_hashes_unchanged() {
        let (_d, mut v, ws, _) = setup();
        let occ = v.list_occurrences().unwrap();
        let ids: Vec<String> = occ.iter().take(3).map(|o| o.id.clone()).collect();
        let h = occ[0].exact_hash.clone();
        let rev = v
            .apply_meta(
                &ws,
                0,
                &MetaOp::SetCategory {
                    occurrence_ids: ids.clone(),
                    category: Some("Bass".into()),
                },
            )
            .unwrap();
        let rev = v
            .apply_meta(
                &ws,
                rev,
                &MetaOp::SetLabels {
                    occurrence_ids: ids.clone(),
                    base: None,
                    prefix: "BS ".into(),
                    suffix: "".into(),
                    number_from: Some(1),
                    clear: false,
                },
            )
            .unwrap();
        let rows = v.occurrences_by_id(&ids).unwrap();
        assert_eq!(rows[0].effective_category, "Bass");
        assert_eq!(rows[1].display_name, "BS Prog 001 2");
        assert_eq!(rows[0].exact_hash, h);
        assert_eq!(v.workspace_view(&ws).unwrap().changed_count, 0);
        let rev = v.undo(&ws, rev).unwrap();
        let rev = v.undo(&ws, rev).unwrap();
        let rows = v.occurrences_by_id(&ids).unwrap();
        assert!(rows[0].manual_category.is_none());
        assert!(v
            .apply_meta(
                &ws,
                rev,
                &MetaOp::SetCategory {
                    occurrence_ids: ids,
                    category: Some("Nope".into())
                }
            )
            .is_err());
    }

    #[test]
    fn sort_by_category() {
        let (_d, mut v, ws, _) = setup();
        let occ = v.list_occurrences().unwrap();
        let rev = v
            .apply_meta(
                &ws,
                0,
                &MetaOp::SetCategory {
                    occurrence_ids: vec![occ[2].id.clone()],
                    category: Some("Bass".into()),
                },
            )
            .unwrap();
        let rev = v
            .apply_meta(
                &ws,
                rev,
                &MetaOp::SetCategory {
                    occurrence_ids: vec![occ[0].id.clone(), occ[1].id.clone()],
                    category: Some("Pad".into()),
                },
            )
            .unwrap();
        v.apply_op(
            &ws,
            rev,
            &WorkspaceOp::SortSelected {
                slots: vec![0, 1, 2],
                key: SortKey::Category,
            },
        )
        .unwrap();
        let view = v.workspace_view(&ws).unwrap();
        let names: Vec<_> = view.slots[..3]
            .iter()
            .map(|s| s.new.as_ref().unwrap().name.clone())
            .collect();
        assert_eq!(names, vec!["Prog 002", "Prog 000", "Prog 001"]);
    }

    #[test]
    fn partial_source_makes_unfinished_workspace() {
        let (_d, mut v) = vault();
        let bank: Vec<Option<Payload>> =
            (0..500).map(|i| Some(synthetic_payload(i, "x"))).collect();
        let bytes = bank_bytes(&bank).unwrap();
        let bytes = &bytes[..1178 * 10];
        let p = preview_import("part.syx", bytes).unwrap();
        let s = v.commit_import(&p, None, bytes).unwrap();
        let ws = v
            .create_workspace_from_source(&s.source_id, "Part")
            .unwrap();
        let view = v.workspace_view(&ws).unwrap();
        assert_eq!(view.empty_count, 490);
        assert!(view.baseline.is_none());
        assert!(v.apply_op(&ws, 0, &WorkspaceOp::ResetToBaseline).is_err());
    }

    #[test]
    fn imported_baseline_rebase_requires_explicit_choices() {
        let (_d, mut v, ws, _) = setup();
        // A "live" read where only slot 7 differs from the imported archive.
        let s = v.begin_read_session("sync", "sim", 1).unwrap();
        for i in 0..500u16 {
            let p = if i == 7 {
                synthetic_payload(9999, "Live 7")
            } else {
                synthetic_payload(i as u32, &format!("Prog {i:03}"))
            };
            v.record_read_slot(&s, i, &p).unwrap();
        }
        let live = v
            .finish_read_session(&s, false, "live", "t")
            .unwrap()
            .snapshot_id
            .unwrap();
        let pv = v.reconcile_preview(&ws, &live).unwrap();
        assert_eq!(pv.len(), 1);
        assert_eq!(pv[0].slot, 7);
        assert_eq!(pv[0].resolution, SlotResolution::Conflict);
        let mut ch = HashMap::new();
        ch.insert(7, ConflictChoice::KeepNew);
        v.apply_rebase(&ws, 0, &live, &ch).unwrap();
        let view = v.workspace_view(&ws).unwrap();
        assert!(view.writable_baseline);
        assert_eq!(view.changed_count, 1);
        assert_eq!(view.slots[7].new.as_ref().unwrap().name, "Prog 007");
        assert!(!view.can_undo, "new history branch after rebase");
    }
}
