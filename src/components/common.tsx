import { CATEGORIES } from "../api/types";

export function CategoryTag({ category, manual, score }: { category: string; manual?: boolean; score?: number }) {
  const i = CATEGORIES.indexOf(category as (typeof CATEGORIES)[number]);
  return (
    <span className={`cat cat-${i < 0 ? 7 : i}`} title={manual ? "Manual category" : score !== undefined ? `Suggestion (heuristic score ${score.toFixed(2)})` : "Suggestion"}>
      {category}
      {manual ? " ●" : ""}
    </span>
  );
}

export function CategoryMenu({ onPick, disabled }: { onPick: (c: string | null) => void; disabled?: boolean }) {
  return (
    <select
      className="catmenu"
      disabled={disabled}
      value=""
      onChange={(e) => {
        const v = e.target.value;
        if (v === "") return;
        onPick(v === "__clear" ? null : v);
        e.target.value = "";
      }}
      title="Set category on the selection (Vault metadata only)"
    >
      <option value="">Category…</option>
      {CATEGORIES.map((c, i) => (
        <option key={c} value={c}>
          {i + 1}. {c}
        </option>
      ))}
      <option value="__clear">Clear override</option>
    </select>
  );
}
