import { describe, expect, it } from "vitest";
import { clearSelection, navigateSelection, selectAll, selectId, visibleSelectionCount } from "./selection";

describe("first-class ID selection", () => {
  const ids = ["a", "b", "c", "d", "e"];
  it("supports replace, Ctrl toggle, and order-aware Shift range", () => {
    let value = selectId(clearSelection(), "b", ids);
    value = selectId(value, "d", ids, { toggle: true });
    expect([...value.selectedIds]).toEqual(["b", "d"]);
    value = selectId(value, "e", ids, { range: true });
    expect([...value.selectedIds]).toEqual(["d", "e"]);
    expect(value.activeId).toBe("e");
  });

  it("selects all result IDs without cloning records and retains hidden IDs", () => {
    const ids10k = Array.from({ length: 10_000 }, (_, index) => `asset-${index}`);
    const started = performance.now();
    const value = selectAll(ids10k, "asset-500");
    const elapsed = performance.now() - started;
    expect(value.selectedIds.size).toBe(10_000);
    expect(visibleSelectionCount(value.selectedIds, ids10k.slice(0, 100))).toBe(100);
    expect(value.activeId).toBe("asset-500");
    expect(elapsed).toBeLessThan(250);
  });

  it("navigates against visible sort order", () => {
    const selected = selectId(clearSelection(), "d", ids);
    expect(navigateSelection(selected, ids, -1).activeId).toBe("c");
  });

  it("records the Milestone 12 selection performance checkpoint", () => {
    const sample = (count: number) => Array.from({ length: count }, (_, index) => `asset-${index}`);
    const measurements: Record<string, number> = {};
    for (const count of [100, 1_000, 10_000]) {
      const idsAtScale = sample(count);
      const started = performance.now();
      const selected = selectAll(idsAtScale, idsAtScale[0]);
      measurements[`select_${count}`] = performance.now() - started;
      expect(selected.selectedIds.size).toBe(count);
    }
    const ids10k = sample(10_000);
    const anchor = selectId(clearSelection(), ids10k[100], ids10k);
    const rangeStarted = performance.now();
    const range = selectId(anchor, ids10k[9_099], ids10k, { range: true });
    measurements.shift_range_9000 = performance.now() - rangeStarted;
    expect(range.selectedIds.size).toBe(9_000);
    console.info(`M12_SELECTION_BENCHMARK ${Object.entries(measurements).map(([name, value]) => `${name}_ms=${value.toFixed(3)}`).join(" ")}`);
  });
});
