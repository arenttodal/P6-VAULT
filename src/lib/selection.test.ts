import { describe, expect, it } from "vitest";
import { clickSelect, emptySelection, isTypingTarget, moveFocus, orderedSelection, pruneToVisible, selectAll } from "./selection";

const order = ["a", "b", "c", "d", "e"];

describe("selection", () => {
  it("click, cmd-click and shift-click use stable ids", () => {
    let s = clickSelect(emptySelection<string>(), order, "b", { meta: false, shift: false });
    s = clickSelect(s, order, "d", { meta: false, shift: true });
    expect([...s.ids].sort()).toEqual(["b", "c", "d"]);
    s = clickSelect(s, order, "c", { meta: true, shift: false });
    expect([...s.ids].sort()).toEqual(["b", "d"]);
    expect(s.focus).toBe("c");
  });

  it("select all includes rows outside any viewport", () => {
    const big = Array.from({ length: 10000 }, (_, i) => `id${i}`);
    expect(selectAll(big, null).ids.size).toBe(10000);
  });

  it("filtering prunes hidden selections; order follows the current display order", () => {
    const s = selectAll(order, "a");
    const pruned = pruneToVisible(s, ["e", "c", "a"]);
    expect([...pruned.ids].sort()).toEqual(["a", "c", "e"]);
    expect(orderedSelection(pruned, ["e", "c", "a"])).toEqual(["e", "c", "a"]);
    expect(pruneToVisible(s, order)).toBe(s);
  });

  it("arrow focus moves and clamps; shift extends", () => {
    let s = moveFocus(emptySelection<string>(), order, 1, false);
    expect(s.focus).toBe("a");
    s = moveFocus(s, order, 1, true);
    expect([...s.ids]).toEqual(["a", "b"]);
    s = moveFocus(s, order, 99, false);
    expect(s.focus).toBe("e");
  });

  it("typing targets are excluded from unmodified shortcuts", () => {
    const input = document.createElement("input");
    const cb = document.createElement("input");
    cb.type = "checkbox";
    expect(isTypingTarget(input)).toBe(true);
    expect(isTypingTarget(cb)).toBe(false);
    expect(isTypingTarget(document.createElement("textarea"))).toBe(true);
    expect(isTypingTarget(document.createElement("div"))).toBe(false);
  });
});
