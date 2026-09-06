import { describe, expect, it } from "vitest";
import { applyAdjustmentPreset, adjustmentPresets } from "./adjustmentPresets";
import { neutralAdjustments } from "../types";

describe("adjustment presets", () => {
  it("keeps crop and orientation when a look is applied", () => {
    const current = { ...neutralAdjustments, cropLeft: .1, cropWidth: .8, rotateQuadrants: 1, straighten: 1.5 };
    const landscape = adjustmentPresets.find((preset) => preset.id === "landscape")!;
    const result = applyAdjustmentPreset(current, landscape);
    expect(result.cropLeft).toBe(.1);
    expect(result.cropWidth).toBe(.8);
    expect(result.rotateQuadrants).toBe(1);
    expect(result.contrast).toBe(10);
  });
});
