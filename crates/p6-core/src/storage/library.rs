//! Sources, occurrences, annotations and protected edit buffers.

use super::*;
use crate::classification::Category;
use crate::library::import::{ImportPreview, OccurrenceKind};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
pub struct ImportSummary {
    pub source_id: String,
    pub file_name: String,
    pub programs: usize,
    pub unique_payloads: usize,
    pub repeated_in_file: usize,
    pub already_in_vault: usize,
    pub excluded: usize,
    pub previously_imported_file: bool,
    pub complete_user_bank: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceRow {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub created_ms: i64,
    pub count: i64,
    pub original_path: Option<String>,
    pub file_hash: Option<String>,
    pub has_archive: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct OccurrenceRow {
    pub id: String,
    pub source_id: String,
    pub source_name: String,
    pub source_kind: String,
    pub message_index: i64,
    pub kind: String,
    pub address: Option<i64>,
    pub stored_name: Option<String>,
    pub display_name: String,
    pub vault_label: Option<String>,
    pub manual_category: Option<String>,
    pub auto_category: String,
    pub auto_score: f64,
    pub effective_category: String,
    pub favorite: bool,
    pub exact_hash: String,
    pub ni_hash: Option<String>,
    pub dup_exact: i64,
    pub dup_name_only: i64,
    pub params_available: bool,
    pub badges: Vec<String>,
    pub noncanonical: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Annotation {
    pub occurrence_id: String,
    pub vault_label: Option<String>,
    pub manual_category: Option<String>,
    pub favorite: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClassificationDetail {
    pub category: String,
    pub score: f64,
    pub reasons: Vec<String>,
    pub classifier_version: u32,
    pub params_available: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProtectedBuffer {
    pub id: String,
    pub blob_hash: String,
    pub occurrence_id: Option<String>,
    pub name: String,
    pub device: Option<String>,
    pub captured_ms: i64,
    pub restored_ms: Option<i64>,
}

pub(crate) const OCC_SELECT: &str = "
SELECT o.id, o.source_id, s.name, s.kind, o.message_index, o.kind, o.orig_address, b.stored_name,
       a.vault_label, a.manual_category, c.category, c.score, COALESCE(a.favorite,0), b.hash, b.ni_hash,
       (SELECT COUNT(*) FROM source_occurrences o2 WHERE o2.blob_hash = o.blob_hash) - 1,
       (SELECT COUNT(*) FROM source_occurrences o3 JOIN patch_blobs b3 ON b3.hash=o3.blob_hash
          WHERE b.ni_hash IS NOT NULL AND b3.ni_hash = b.ni_hash AND b3.hash <> b.hash),
       c.params_available, c.badges, o.noncanonical
FROM source_occurrences o
JOIN sources s ON s.id = o.source_id
JOIN patch_blobs b ON b.hash = o.blob_hash
LEFT JOIN patch_annotations a ON a.occurrence_id = o.id
LEFT JOIN classifications c ON c.blob_hash = b.hash AND c.classifier_version = 1";

pub(crate) fn occ_row(r: &rusqlite::Row) -> rusqlite::Result<OccurrenceRow> {
    let stored_name: Option<String> = r.get(7)?;
    let label: Option<String> = r.get(8)?;
    let manual: Option<String> = r.get(9)?;
    let auto: Option<String> = r.get(10)?;
    let address: Option<i64> = r.get(6)?;
    let source_name: String = r.get(2)?;
    let mi: i64 = r.get(4)?;
    let kind: String = r.get(5)?;
    let fallback = match address {
        Some(a) => format!("{source_name} #{a:03}"),
        None => format!("{source_name} (edit buffer #{mi})"),
    };
    let display_name = label.clone().filter(|s| !s.trim().is_empty()).or_else(|| stored_name.clone()).unwrap_or(fallback);
    let auto = auto.unwrap_or_else(|| "Other".into());
    let badges: Option<String> = r.get(18)?;
    Ok(OccurrenceRow {
        id: r.get(0)?,
        source_id: r.get(1)?,
        source_name,
        source_kind: r.get(3)?,
        message_index: mi,
        kind,
        address,
        stored_name,
        display_name,
        vault_label: label,
        effective_category: manual.clone().unwrap_or_else(|| auto.clone()),
        manual_category: manual,
        auto_category: auto,
        auto_score: r.get::<_, Option<f64>>(11)?.unwrap_or(0.0),
        favorite: r.get::<_, i64>(12)? != 0,
        exact_hash: r.get(13)?,
        ni_hash: r.get(14)?,
        dup_exact: r.get(15)?,
        dup_name_only: r.get(16)?,
        params_available: r.get::<_, Option<bool>>(17)?.unwrap_or(false),
        badges: badges.and_then(|b| serde_json::from_str(&b).ok()).unwrap_or_default(),
        noncanonical: r.get(19)?,
    })
}

impl Vault {
    /// Transactionally commit a previewed import. Retains an unmodified archive copy.
    pub fn commit_import(&mut self, preview: &ImportPreview, original_path: Option<&str>, raw: &[u8]) -> VResult<ImportSummary> {
        if crate::library::import::file_hash(raw) != preview.file_hash {
            return Err(VaultError::Invalid("file changed between preview and import".into()));
        }
        let archive = self.archives_dir().join(format!("{}.syx", preview.file_hash));
        if !archive.exists() {
            crate::library::export::write_verified(&archive, raw, &crate::library::export::Expected::Raw).map_err(|e| VaultError::Io(e.to_string()))?;
        }
        let previously: bool = self.conn.query_row("SELECT EXISTS(SELECT 1 FROM sources WHERE file_hash=?1)", [&preview.file_hash], |r| r.get(0))?;
        let tx = self.conn.transaction()?;
        let sid = insert_source(&tx, &preview.file_name, "file", original_path, Some(&archive.to_string_lossy()), Some(&preview.file_hash), None)?;
        let mut already = 0;
        for o in &preview.occurrences {
            let (hash, new) = insert_blob(&tx, &o.payload)?;
            if !new {
                already += 1;
            }
            let kind = match o.kind {
                OccurrenceKind::Program => "program",
                OccurrenceKind::EditBuffer => "edit_buffer",
            };
            insert_occurrence(&tx, &sid, &hash, o.message_index, Some(o.byte_offset), kind, o.address.map(|a| a.absolute()), Some(&o.frame), o.noncanonical)?;
        }
        tx.commit()?;
        Ok(ImportSummary {
            source_id: sid,
            file_name: preview.file_name.clone(),
            programs: preview.occurrences.len(),
            unique_payloads: preview.unique_payloads,
            repeated_in_file: preview.repeated_in_file,
            already_in_vault: already,
            excluded: preview.excluded.len(),
            previously_imported_file: previously,
            complete_user_bank: preview.complete_user_bank().is_some(),
        })
    }

    /// Count of payloads in a preview that already exist in the Vault (from other sources).
    pub fn count_known_payloads(&self, preview: &ImportPreview) -> VResult<usize> {
        let mut st = self.conn.prepare_cached("SELECT EXISTS(SELECT 1 FROM patch_blobs WHERE hash=?1)")?;
        let mut n = 0;
        let mut seen = std::collections::HashSet::new();
        for o in &preview.occurrences {
            let h = o.payload.exact_hash();
            if seen.insert(h.clone()) && st.query_row([&h], |r| r.get::<_, bool>(0))? {
                n += 1;
            }
        }
        Ok(n)
    }

    pub fn list_sources(&self) -> VResult<Vec<SourceRow>> {
        let mut st = self.conn.prepare(
            "SELECT s.id, s.name, s.kind, s.created_ms, (SELECT COUNT(*) FROM source_occurrences o WHERE o.source_id=s.id), s.original_path, s.file_hash, s.archive_path IS NOT NULL
             FROM sources s ORDER BY s.created_ms, s.rowid",
        )?;
        let rows = st
            .query_map([], |r| {
                Ok(SourceRow {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    kind: r.get(2)?,
                    created_ms: r.get(3)?,
                    count: r.get(4)?,
                    original_path: r.get(5)?,
                    file_hash: r.get(6)?,
                    has_archive: r.get(7)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }

    pub fn source_archive(&self, source_id: &str) -> VResult<(String, PathBuf)> {
        let (name, path): (String, Option<String>) =
            self.conn.query_row("SELECT name, archive_path FROM sources WHERE id=?1", [source_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        let p = path.ok_or_else(|| VaultError::NotFound("this source has no retained archive".into()))?;
        Ok((name, PathBuf::from(p)))
    }

    /// Lightweight library rows (no payloads), in stable source/message order.
    pub fn list_occurrences(&self) -> VResult<Vec<OccurrenceRow>> {
        let sql = format!("{OCC_SELECT} ORDER BY s.created_ms, s.rowid, o.message_index, o.rowid");
        let mut st = self.conn.prepare(&sql)?;
        let rows = st.query_map([], occ_row)?.collect::<Result<_, _>>()?;
        Ok(rows)
    }

    pub fn occurrences_by_id(&self, ids: &[String]) -> VResult<Vec<OccurrenceRow>> {
        let sql = format!("{OCC_SELECT} WHERE o.id = ?1");
        let mut st = self.conn.prepare_cached(&sql)?;
        ids.iter().map(|id| st.query_row([id], occ_row).map_err(VaultError::from)).collect()
    }

    pub fn classification_detail(&self, blob_hash: &str) -> VResult<ClassificationDetail> {
        Ok(self.conn.query_row(
            "SELECT category, score, evidence, classifier_version, params_available FROM classifications WHERE blob_hash=?1 ORDER BY classifier_version DESC LIMIT 1",
            [blob_hash],
            |r| {
                Ok(ClassificationDetail {
                    category: r.get(0)?,
                    score: r.get(1)?,
                    reasons: serde_json::from_str(&r.get::<_, String>(2)?).unwrap_or_default(),
                    classifier_version: r.get(3)?,
                    params_available: r.get(4)?,
                })
            },
        )?)
    }

    pub(crate) fn annotations(&self, ids: &[String]) -> VResult<Vec<Annotation>> {
        let mut st = self.conn.prepare_cached("SELECT vault_label, manual_category, favorite FROM patch_annotations WHERE occurrence_id=?1")?;
        ids.iter()
            .map(|id| {
                let a = st.query_row([id], |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, i64>(2)? != 0))).optional()?;
                Ok(match a {
                    Some((l, c, f)) => Annotation { occurrence_id: id.clone(), vault_label: l, manual_category: c, favorite: f },
                    None => Annotation { occurrence_id: id.clone(), ..Default::default() },
                })
            })
            .collect()
    }

    pub(crate) fn write_annotations(tx: &Transaction, anns: &[Annotation]) -> VResult<()> {
        for a in anns {
            if let Some(c) = &a.manual_category {
                if Category::from_label(c).is_none() {
                    return Err(VaultError::Invalid(format!("unknown category '{c}'")));
                }
            }
            tx.execute(
                "INSERT INTO patch_annotations(occurrence_id,vault_label,manual_category,favorite,updated_ms) VALUES(?1,?2,?3,?4,?5)
                 ON CONFLICT(occurrence_id) DO UPDATE SET vault_label=excluded.vault_label, manual_category=excluded.manual_category, favorite=excluded.favorite, updated_ms=excluded.updated_ms",
                params![a.occurrence_id, a.vault_label, a.manual_category, a.favorite, now_ms()],
            )?;
        }
        Ok(())
    }

    /// Store a captured edit buffer as a protected, restorable item.
    pub fn protect_edit_buffer(&mut self, p: &Payload, device: &str) -> VResult<ProtectedBuffer> {
        let tx = self.conn.transaction()?;
        let (hash, _) = insert_blob(&tx, p)?;
        let name = format!("Protected edit buffer {}", crate::util::file_timestamp(now_ms()));
        let sid = insert_source(&tx, &name, "edit_buffer_capture", None, None, None, Some(device))?;
        let occ = insert_occurrence(&tx, &sid, &hash, 0, None, "edit_buffer", None, None, false)?;
        let id = new_id();
        let now = now_ms();
        tx.execute("INSERT INTO protected_buffers(id,blob_hash,occurrence_id,device,captured_ms) VALUES(?1,?2,?3,?4,?5)", params![id, hash, occ, device, now])?;
        tx.commit()?;
        Ok(ProtectedBuffer {
            id,
            blob_hash: hash,
            occurrence_id: Some(occ),
            name: p.display_name().unwrap_or(name),
            device: Some(device.into()),
            captured_ms: now,
            restored_ms: None,
        })
    }

    pub fn protected_buffers(&self) -> VResult<Vec<ProtectedBuffer>> {
        let mut st = self.conn.prepare(
            "SELECT p.id, p.blob_hash, p.occurrence_id, b.stored_name, p.device, p.captured_ms, p.restored_ms FROM protected_buffers p JOIN patch_blobs b ON b.hash=p.blob_hash ORDER BY p.captured_ms DESC",
        )?;
        let rows = st
            .query_map([], |r| {
                Ok(ProtectedBuffer {
                    id: r.get(0)?,
                    blob_hash: r.get(1)?,
                    occurrence_id: r.get(2)?,
                    name: r.get::<_, Option<String>>(3)?.unwrap_or_else(|| "(unnamed)".into()),
                    device: r.get(4)?,
                    captured_ms: r.get(5)?,
                    restored_ms: r.get(6)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }

    pub fn mark_buffer_restored(&self, id: &str) -> VResult<()> {
        self.conn.execute("UPDATE protected_buffers SET restored_ms=?2 WHERE id=?1", params![id, now_ms()])?;
        Ok(())
    }

    /// Payloads of a source as (occurrence id, address, payload), for bank construction.
    pub fn source_programs(&self, source_id: &str) -> VResult<Vec<(String, Option<u16>, String)>> {
        let mut st = self.conn.prepare("SELECT id, orig_address, blob_hash FROM source_occurrences WHERE source_id=?1 ORDER BY message_index")?;
        let rows = st.query_map([source_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<_, _>>()?;
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::super::testutil::vault;
    use crate::library::import::preview_import;
    use crate::protocol::messages::program_file_frame;
    use crate::protocol::payload::synthetic_payload;
    use crate::slot::StoredAddress;

    #[test]
    fn import_twice_keeps_provenance() {
        let (_d, mut v) = vault();
        let mut data = Vec::new();
        for i in 0..3u16 {
            data.extend(program_file_frame(StoredAddress::from_absolute(i).unwrap(), &synthetic_payload(i as u32, &format!("Bass {i}"))));
        }
        let p = preview_import("a.syx", &data).unwrap();
        let s1 = v.commit_import(&p, Some("/x/a.syx"), &data).unwrap();
        assert_eq!(s1.already_in_vault, 0);
        assert_eq!(v.count_known_payloads(&p).unwrap(), 3);
        let s2 = v.commit_import(&p, Some("/x/a.syx"), &data).unwrap();
        assert_eq!(s2.already_in_vault, 3);
        assert!(s2.previously_imported_file);
        let rows = v.list_occurrences().unwrap();
        assert_eq!(rows.len(), 6);
        assert_eq!(rows[0].dup_exact, 1);
        assert_eq!(rows[0].display_name, "Bass 0");
        assert_eq!(v.list_sources().unwrap().len(), 2);
        let (_, path) = v.source_archive(&s1.source_id).unwrap();
        assert_eq!(std::fs::read(path).unwrap(), data);
    }

    #[test]
    fn import_mixed_file_with_foreign_messages() {
        let (_d, mut v) = vault();
        let mut data = vec![0xF0, 0x42, 0x30, 0x00, 0xF7];
        data.extend(program_file_frame(StoredAddress::from_absolute(3).unwrap(), &synthetic_payload(3, "x")));
        let p = preview_import("m.syx", &data).unwrap();
        let s = v.commit_import(&p, None, &data).unwrap();
        assert_eq!(s.programs, 1);
        assert_eq!(s.excluded, 1);
        let (_, path) = v.source_archive(&s.source_id).unwrap();
        assert_eq!(std::fs::read(path).unwrap(), data);
    }

    #[test]
    fn blobs_immutable() {
        let (_d, mut v) = vault();
        let data = program_file_frame(StoredAddress::from_absolute(0).unwrap(), &synthetic_payload(1, "x"));
        let p = preview_import("a.syx", &data).unwrap();
        v.commit_import(&p, None, &data).unwrap();
        assert!(v.conn.execute("UPDATE patch_blobs SET payload=zeroblob(1024)", []).is_err());
    }
}
