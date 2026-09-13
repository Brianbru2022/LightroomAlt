import { describe, expect, it } from "vitest";
import { neutralAdjustments, neutralAdvancedDevelop } from "../types";
import { canonicalToDisplay, createMask, displayToCanonical } from "./masks";

describe("mask coordinates", () => {
  it("round-trips canonical points through crop, rotation and flips", () => {
    const settings = { ...neutralAdjustments, cropLeft: .1, cropTop: .2, cropWidth: .7, cropHeight: .6, rotateQuadrants: 1, horizontalFlip: true, straighten: 7 };
    const point = { x: .43, y: .61 }; const restored = displayToCanonical(canonicalToDisplay(point, settings, 1.5), settings, 1.5);
    expect(restored.x).toBeCloseTo(point.x, 5); expect(restored.y).toBeCloseTo(point.y, 5);
  });

  it("creates compact resolution-independent geometry", () => {
    for (const kind of ["linear", "radial", "brush"] as const) {
      const mask = createMask(kind, 1); expect(JSON.stringify(mask)).not.toMatch(/width|height|bitmap|data:/i);
    }
  });

  it("round-trips canonical points through optical correction and geometry", () => {
    const settings={...neutralAdjustments,cropLeft:.05,cropWidth:.9,straighten:-3};
    const lens={...neutralAdvancedDevelop().lens,enabled:true,manualDistortion:55,constrainCrop:true};
    for(const point of [{x:.12,y:.18},{x:.5,y:.5},{x:.84,y:.72}]){
      const restored=displayToCanonical(canonicalToDisplay(point,settings,1.5,lens),settings,1.5,lens);
      expect(restored.x).toBeCloseTo(point.x,4);expect(restored.y).toBeCloseTo(point.y,4);
    }
  });
});
