import type { OccurrenceRow } from "../api/types";

export type LibSort = "source" | "name" | "category" | "address";
export type DupFilter = "all" | "exact" | "name";

/** Hardware-read sources that are hidden from "All sounds" (still reachable per source). */
export const ARCHIVAL_KINDS = new Set(["backup", "partial_read"]);
const HARDWARE_STATE_KINDS = new Set(["live_snapshot", "post_write"]);

/**
 * Sources shown in "All sounds": imported files, protected edit buffers and the newest
 * complete read of the synth (sync or post-write verification). Older reads, backups and verification reads stay in the Vault
 * and are listed under "Hardware reads & backups".
 */
export function defaultVisibleSources(sources: { id: string; kind: string; created_ms: number }[]): Set<string> {
  const live = sources.filter((s) => HARDWARE_STATE_KINDS.has(s.kind)).sort((a, b) => b.created_ms - a.created_ms);
  const keep = new Set(sources.filter((s) => !HARDWARE_STATE_KINDS.has(s.kind) && !ARCHIVAL_KINDS.has(s.kind)).map((s) => s.id));
  if (live[0]) keep.add(live[0].id);
  return keep;
}

export interface LibraryFilter {
  search: string;
  sourceId: string | null;
  category: string | null;
  favoritesOnly: boolean;
  dup: DupFilter;
  unclassifiedOnly: boolean;
  sort: LibSort;
}

export const defaultFilter: LibraryFilter = {
  search: "",
  sourceId: null,
  category: null,
  favoritesOnly: false,
  dup: "all",
  unclassifiedOnly: false,
  sort: "source",
};

export function formatAddress(r: Pick<OccurrenceRow, "address" | "message_index">): string {
  return r.address == null ? `Edit buffer #${r.message_index}` : String(r.address).padStart(3, "0");
}

const CAT_ORDER = ["Bass", "Lead", "Pad", "Keys", "Pluck", "Arp / Sequence", "FX / Texture", "Other"];

/**
 * Restrict to sources in scope and recount duplicates within that scope, so repeated
 * hardware reads of the same bank don't make everything look duplicated.
 */
export function scopeRows(rows: OccurrenceRow[], f: Pick<LibraryFilter, "sourceId">, visibleSources?: Set<string>): OccurrenceRow[] {
  const inScope = rows.filter((r) => (f.sourceId ? true : !visibleSources || visibleSources.has(r.source_id)));
  const exact = new Map<string, number>();
  const byNi = new Map<string, Set<string>>();
  for (const r of inScope) {
    exact.set(r.exact_hash, (exact.get(r.exact_hash) ?? 0) + 1);
    if (r.ni_hash) {
      const set = byNi.get(r.ni_hash) ?? new Set<string>();
      set.add(r.exact_hash);
      byNi.set(r.ni_hash, set);
    }
  }
  return inScope.map((r) => {
    const dup_exact = (exact.get(r.exact_hash) ?? 1) - 1;
    let dup_name_only = 0;
    if (r.ni_hash) for (const h of byNi.get(r.ni_hash) ?? []) if (h !== r.exact_hash) dup_name_only += exact.get(h) ?? 0;
    return dup_exact === r.dup_exact && dup_name_only === r.dup_name_only ? r : { ...r, dup_exact, dup_name_only };
  });
}

/** Filter + stable sort. Ties fall back to source/message order (the input order). */
export function filterLibrary(allRows: OccurrenceRow[], f: LibraryFilter, visibleSources?: Set<string>): OccurrenceRow[] {
  const rows = scopeRows(allRows, f, visibleSources);
  const q = f.search.trim().toLowerCase();
  const terms = q ? q.split(/\s+/) : [];
  const out = rows
    .map((r, idx) => ({ r, idx }))
    .filter(({ r }) => {
      if (f.sourceId && r.source_id !== f.sourceId) return false;
      if (f.category && r.effective_category !== f.category) return false;
      if (f.favoritesOnly && !r.favorite) return false;
      if (f.dup === "exact" && r.dup_exact === 0) return false;
      if (f.dup === "name" && r.dup_name_only === 0) return false;
      if (f.unclassifiedOnly && (r.manual_category || r.auto_category !== "Other")) return false;
      if (terms.length) {
        const hay = [r.display_name, r.stored_name ?? "", r.vault_label ?? "", r.effective_category, r.source_name, formatAddress(r)]
          .join(" ")
          .toLowerCase();
        if (!terms.every((t) => hay.includes(t))) return false;
      }
      return true;
    });
  const cmp = (a: { r: OccurrenceRow; idx: number }, b: { r: OccurrenceRow; idx: number }): number => {
    let c = 0;
    switch (f.sort) {
      case "name":
        c = a.r.display_name.localeCompare(b.r.display_name, undefined, { sensitivity: "base", numeric: true });
        break;
      case "category":
        c = CAT_ORDER.indexOf(a.r.effective_category) - CAT_ORDER.indexOf(b.r.effective_category);
        if (c === 0) c = a.r.display_name.localeCompare(b.r.display_name, undefined, { sensitivity: "base", numeric: true });
        break;
      case "address":
        c = (a.r.address ?? 10000) - (b.r.address ?? 10000);
        break;
      default:
        c = 0;
    }
    return c !== 0 ? c : a.idx - b.idx;
  };
  return out.sort(cmp).map((x) => x.r);
}
