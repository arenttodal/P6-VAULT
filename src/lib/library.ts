import type { OccurrenceRow } from "../api/types";

export type LibSort = "source" | "name" | "category" | "address";
export type DupFilter = "all" | "exact" | "name";

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

/** Filter + stable sort. Ties fall back to source/message order (the input order). */
export function filterLibrary(rows: OccurrenceRow[], f: LibraryFilter): OccurrenceRow[] {
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
