import { neutralAdjustments, type BasicAdjustments } from "../types";

export type AdjustmentPreset = { id: string; label: string; values: Partial<BasicAdjustments> };

export const adjustmentPresets: AdjustmentPreset[] = [
  { id: "natural", label: "Natural", values: {} },
  { id: "bright", label: "Bright", values: { exposure: 0.3, shadows: 18, highlights: -12, colourBoost: 6 } },
  { id: "warm", label: "Warm", values: { lightBalance: 16, tint: 3, contrast: 5, colourBoost: 8 } },
  { id: "cool", label: "Cool", values: { lightBalance: -14, tint: -2, highlights: -10, clarity: 5 } },
  { id: "landscape", label: "Landscape", values: { contrast: 10, dehaze: 7, colourBoost: 16, clarity: 8 } },
  { id: "soft", label: "Soft", values: { contrast: -10, highlights: -15, shadows: 12, texture: -8 } },
  { id: "mono", label: "Mono", values: { saturation: -100, contrast: 12, clarity: 5 } },
];

export function applyAdjustmentPreset(current: BasicAdjustments, preset: AdjustmentPreset): BasicAdjustments {
  return { ...neutralAdjustments, ...preset.values, cropLeft: current.cropLeft, cropTop: current.cropTop, cropWidth: current.cropWidth, cropHeight: current.cropHeight, rotateQuadrants: current.rotateQuadrants, straighten: current.straighten };
}
