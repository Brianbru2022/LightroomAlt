import { neutralLocalAdjustments, type BasicAdjustments, type DevelopMask, type MaskPoint } from "../types";

export type MaskTool = "linear" | "radial" | "brush" | "erase" | "add" | "subtract" | null;

export const createMask = (kind: "linear" | "radial" | "brush", index: number): DevelopMask => ({
  id: crypto.randomUUID(),
  name: `${kind === "linear" ? "Linear gradient" : kind === "radial" ? "Radial gradient" : "Brush"} ${index}`,
  enabled: true,
  inverted: false,
  opacity: 1,
  feather: kind === "brush" ? .65 : .6,
  geometry: kind === "linear"
    ? { kind, start: { x: .25, y: .5 }, end: { x: .75, y: .5 } }
    : kind === "radial"
      ? { kind, centre: { x: .5, y: .5 }, radiusX: .25, radiusY: .2, rotation: 0 }
      : { kind, strokes: [] },
  adjustments: { ...neutralLocalAdjustments },
});

const rotateForward = (point: MaskPoint, quadrants: number): MaskPoint => {
  let current = point;
  for (let index = 0; index < quadrants; index += 1) current = { x: 1 - current.y, y: current.x };
  return current;
};

const rotateBack = (point: MaskPoint, quadrants: number): MaskPoint => {
  let current = point;
  for (let index = 0; index < quadrants; index += 1) current = { x: current.y, y: 1 - current.x };
  return current;
};

export function canonicalToDisplay(point: MaskPoint, settings: BasicAdjustments, sourceAspect = 1): MaskPoint {
  let current = { x: settings.horizontalFlip ? 1 - point.x : point.x, y: settings.verticalFlip ? 1 - point.y : point.y };
  current = rotateForward(current, settings.rotateQuadrants);
  if (Math.abs(settings.straighten) >= .01) {
    const aspect = settings.rotateQuadrants % 2 ? 1 / sourceAspect : sourceAspect;
    const angle = settings.straighten * Math.PI / 180; const dx = current.x - .5; const dy = current.y - .5;
    current = { x: Math.cos(angle) * dx - Math.sin(angle) * dy / aspect + .5, y: Math.sin(angle) * dx * aspect + Math.cos(angle) * dy + .5 };
  }
  return { x: (current.x - settings.cropLeft) / settings.cropWidth, y: (current.y - settings.cropTop) / settings.cropHeight };
}

export function displayToCanonical(point: MaskPoint, settings: BasicAdjustments, sourceAspect = 1): MaskPoint {
  let current = { x: point.x * settings.cropWidth + settings.cropLeft, y: point.y * settings.cropHeight + settings.cropTop };
  if (Math.abs(settings.straighten) >= .01) {
    const aspect = settings.rotateQuadrants % 2 ? 1 / sourceAspect : sourceAspect;
    const angle = -settings.straighten * Math.PI / 180; const dx = current.x - .5; const dy = current.y - .5;
    current = { x: Math.cos(angle) * dx - Math.sin(angle) * dy / aspect + .5, y: Math.sin(angle) * dx * aspect + Math.cos(angle) * dy + .5 };
  }
  current = rotateBack(current, settings.rotateQuadrants);
  current = { x: settings.horizontalFlip ? 1 - current.x : current.x, y: settings.verticalFlip ? 1 - current.y : current.y };
  return { x: Math.max(0, Math.min(1, current.x)), y: Math.max(0, Math.min(1, current.y)) };
}

export function outputAspect(width: number, height: number, settings: BasicAdjustments) {
  const rotated = settings.rotateQuadrants % 2 !== 0;
  const orientedWidth = rotated ? height : width; const orientedHeight = rotated ? width : height;
  return (orientedWidth * settings.cropWidth) / Math.max(1, orientedHeight * settings.cropHeight);
}
