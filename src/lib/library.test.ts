import { describe, expect, it } from "vitest";
import type { OccurrenceRow } from "../api/types";
import { defaultFilter, defaultVisibleSources, filterLibrary, formatAddress } from "./library";

function row(p: Partial<OccurrenceRow>): OccurrenceRow {
  return {
    id: "x",
    source_id: "s1",
    source_name: "old.syx",
    source_kind: "file",
    message_index: 0,
    kind: "program",
    address: 0,
    stored_name: "Name",
    display_name: "Name",
    vault_label: null,
    manual_category: null,
    auto_category: "Other",
    auto_score: 0,
    effective_category: "Other",
    favorite: false,
    exact_hash: "h",
    ni_hash: "n",
    dup_exact: 0,
    dup_name_only: 0,
    params_available: true,
    badges: [],
    noncanonical: false,
    format_version: 1,
    ...p,
  };
}

const rows = [
  row({ id: "1", display_name: "Warm Pad", effective_category: "Pad", exact_hash: "a", ni_hash: "A" }),
  row({ id: "2", display_name: "acid bass", effective_category: "Bass", exact_hash: "b", ni_hash: "B", address: 120 }),
  row({ id: "3", display_name: "Bright Lead", effective_category: "Lead", exact_hash: "a", ni_hash: "A", favorite: true }),
  row({ id: "4", display_name: "Renamed Pad", effective_category: "Pad", exact_hash: "c", ni_hash: "A", address: null, message_index: 7 }),
  row({ id: "5", display_name: "Old read", source_id: "hw-old", exact_hash: "a", ni_hash: "A" }),
];

describe("library filtering", () => {
  it("searches name, category, source and address", () => {
    expect(filterLibrary(rows, { ...defaultFilter, search: "pad" }).map((r) => r.id)).toEqual(["1", "4"]);
    expect(filterLibrary(rows, { ...defaultFilter, search: "120" }).map((r) => r.id)).toEqual(["2"]);
    expect(filterLibrary(rows, { ...defaultFilter, search: "old.syx bass" }).map((r) => r.id)).toEqual(["2"]);
  });

  it("stable sort by name falls back to source order", () => {
    expect(filterLibrary(rows.slice(0, 4), { ...defaultFilter, sort: "name" }).map((r) => r.display_name)).toEqual(["acid bass", "Bright Lead", "Renamed Pad", "Warm Pad"]);
    expect(filterLibrary(rows.slice(0, 4), { ...defaultFilter, sort: "category" }).map((r) => r.id)).toEqual(["2", "3", "4", "1"]);
  });

  it("duplicates are counted within scope (older hardware reads hidden)", () => {
    const vis = new Set(["s1"]);
    const out = filterLibrary(rows, { ...defaultFilter, dup: "exact" }, vis);
    expect(out.map((r) => r.id)).toEqual(["1", "3"]);
    expect(out[0].dup_exact).toBe(1);
    const nameOnly = filterLibrary(rows, { ...defaultFilter, dup: "name" }, vis);
    expect(nameOnly.map((r) => r.id)).toEqual(["1", "3", "4"]);
    // Selecting the hidden source explicitly shows it
    expect(filterLibrary(rows, { ...defaultFilter, sourceId: "hw-old" }, vis).map((r) => r.id)).toEqual(["5"]);
  });

  it("edit-buffer rows never show an invented slot", () => {
    expect(formatAddress(rows[3])).toBe("Edit buffer #7");
    expect(formatAddress(rows[1])).toBe("120");
  });

  it("default visible sources: files + newest live read only", () => {
    const v = defaultVisibleSources([
      { id: "f", kind: "file", created_ms: 1 },
      { id: "l1", kind: "live_snapshot", created_ms: 2 },
      { id: "l2", kind: "live_snapshot", created_ms: 3 },
      { id: "b", kind: "backup", created_ms: 4 },
      { id: "e", kind: "edit_buffer_capture", created_ms: 5 },
    ]);
    expect([...v].sort()).toEqual(["e", "f", "l2"]);
    const after = defaultVisibleSources([
      { id: "l2", kind: "live_snapshot", created_ms: 3 },
      { id: "p", kind: "post_write", created_ms: 6 },
    ]);
    expect([...after]).toEqual(["p"]);
  });
});
