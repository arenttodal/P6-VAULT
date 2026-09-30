import { useMemo } from "react";
import { api } from "../api/backend";
import { CATEGORIES } from "../api/types";
import { useApp } from "../stores/app";

export function Sidebar() {
  const { sources, occurrences, filter, setFilter, workspaces, workspace, openDialog, refreshWorkspace, error, toast } = useApp();
  const counts = useMemo(() => {
    const c: Record<string, number> = {};
    let fav = 0,
      dupE = 0,
      dupN = 0,
      uncl = 0;
    for (const o of occurrences) {
      c[o.effective_category] = (c[o.effective_category] ?? 0) + 1;
      if (o.favorite) fav++;
      if (o.dup_exact > 0) dupE++;
      if (o.dup_name_only > 0) dupN++;
      if (!o.manual_category && o.auto_category === "Other") uncl++;
    }
    return { c, fav, dupE, dupN, uncl };
  }, [occurrences]);

  const buildFrom = async (sourceId: string, name: string) => {
    try {
      const ws = await api().createWorkspaceFromSource(sourceId, name);
      await api().setActiveWorkspace(ws);
      await refreshWorkspace();
      toast("success", `New bank created from ${name}.`);
    } catch (e) {
      error(e);
    }
  };

  const exportOriginal = async (id: string, name: string) => {
    const p = await api().saveFile(name.endsWith(".syx") ? name : `${name}.syx`);
    if (!p) return;
    try {
      await api().exportSource(id, p);
      toast("success", "Original file exported unchanged.");
    } catch (e) {
      error(e);
    }
  };

  const item = (label: string, active: boolean, onClick: () => void, count?: number, title?: string) => (
    <li className={active ? "active" : ""} onClick={onClick} title={title}>
      <span className="label">{label}</span>
      {count !== undefined && <span className="count">{count}</span>}
    </li>
  );

  const clearFilters = () => setFilter({ sourceId: null, category: null, favoritesOnly: false, dup: "all", unclassifiedOnly: false });
  const noneActive = !filter.sourceId && !filter.category && !filter.favoritesOnly && filter.dup === "all" && !filter.unclassifiedOnly;

  return (
    <aside className="sidebar">
      <h3>Library</h3>
      <ul>
        {item("All sounds", noneActive, clearFilters, occurrences.length)}
        {item("Favorites", filter.favoritesOnly, () => setFilter({ favoritesOnly: !filter.favoritesOnly }), counts.fav)}
        {item("Exact duplicates", filter.dup === "exact", () => setFilter({ dup: filter.dup === "exact" ? "all" : "exact" }), counts.dupE, "Identical payload in more than one place")}
        {item("Same except name", filter.dup === "name", () => setFilter({ dup: filter.dup === "name" ? "all" : "name" }), counts.dupN, "Identical payload apart from the 20 name bytes")}
        {item("Unclassified", filter.unclassifiedOnly, () => setFilter({ unclassifiedOnly: !filter.unclassifiedOnly }), counts.uncl)}
      </ul>
      <h3>Categories</h3>
      <ul>
        {CATEGORIES.map((c, i) => (
          <li key={c} className={filter.category === c ? "active" : ""} onClick={() => setFilter({ category: filter.category === c ? null : c })} title={`Shortcut ${i + 1} assigns this category`}>
            <span className={`cat-dot cat-${i}`} aria-hidden />
            <span className="label">{c}</span>
            <span className="count">{counts.c[c] ?? 0}</span>
          </li>
        ))}
      </ul>
      <h3>Sources</h3>
      <ul className="sources">
        {sources.length === 0 && <li className="hint">Import .syx files or sync from the synth.</li>}
        {sources.map((s) => (
          <li key={s.id} className={filter.sourceId === s.id ? "active" : ""} onClick={() => setFilter({ sourceId: filter.sourceId === s.id ? null : s.id })} title={s.original_path ?? s.name}>
            <span className="label">{s.name}</span>
            <span className="count">{s.count}</span>
            <span className="row-actions">
              {s.kind === "file" && (
                <button
                  className="mini"
                  title="Make a New bank from this file's addresses (no MIDI)"
                  onClick={(e) => {
                    e.stopPropagation();
                    void buildFrom(s.id, s.name);
                  }}
                >
                  Bank
                </button>
              )}
              {s.has_archive && (
                <button
                  className="mini"
                  title="Export the original file unchanged"
                  onClick={(e) => {
                    e.stopPropagation();
                    void exportOriginal(s.id, s.name);
                  }}
                >
                  ⇩
                </button>
              )}
            </span>
          </li>
        ))}
      </ul>
      <h3>Banks</h3>
      <ul>
        {workspaces.map((w) => (
          <li
            key={w.id}
            className={workspace?.id === w.id ? "active" : ""}
            onClick={async () => {
              try {
                await api().setActiveWorkspace(w.id);
                await refreshWorkspace();
              } catch (e) {
                error(e);
              }
            }}
          >
            <span className="label">{w.name}</span>
          </li>
        ))}
      </ul>
      <h3>Safety</h3>
      <ul>
        {item("History & backups", false, () => openDialog({ kind: "history" }))}
        {item("MIDI diagnostics", false, () => openDialog({ kind: "diagnostics" }))}
      </ul>
    </aside>
  );
}
