import { neutralLocalAdjustments, type BasicAdjustments, type DevelopMask, type LensCorrections, type MaskPoint } from "../types";

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

const profileDistortion: Record<string, [number, number, number]> = {
  "lensfun-canon-650d-ef50-f18-aps-c": [-.00149, 0, 0],
  "lensfun-nikon-d750-af50-f18d": [.00139, -.00804, .00877],
};
const opticalSource = (point: MaskPoint, lens: LensCorrections | undefined, aspect: number): MaskPoint => {
  if (!lens?.enabled) return point;
  const profile = lens.profileMode !== "off" && lens.profileId ? profileDistortion[lens.profileId] : undefined;
  const amount = lens.profileAmount / 100;
  const coefficients: [number,number,number] = [lens.manualDistortion / 100 * .18 + (profile?.[0] ?? 0) * amount, (profile?.[1] ?? 0) * amount, (profile?.[2] ?? 0) * amount];
  let nx=(point.x-.5)*2*aspect, ny=(point.y-.5)*2; const r2=nx*nx+ny*ny; const radial=1+coefficients[0]*r2+coefficients[1]*r2*r2+coefficients[2]*r2*r2*r2; const crop=lens.constrainCrop?1/(1+coefficients.reduce((sum,value)=>sum+Math.abs(value),0)*1.4):1; nx*=radial*crop; ny*=radial*crop;
  return {x:nx/aspect/2+.5,y:ny/2+.5};
};
const opticalDisplay = (point: MaskPoint, lens: LensCorrections | undefined, aspect: number): MaskPoint => {
  let result={...point}; for(let index=0;index<8;index+=1){const mapped=opticalSource(result,lens,aspect);result={x:result.x+point.x-mapped.x,y:result.y+point.y-mapped.y};} return result;
};

export function canonicalToDisplay(point: MaskPoint, settings: BasicAdjustments, sourceAspect = 1, lens?: LensCorrections): MaskPoint {
  const optical = opticalDisplay(point, lens, sourceAspect);
  let current = { x: settings.horizontalFlip ? 1 - optical.x : optical.x, y: settings.verticalFlip ? 1 - optical.y : optical.y };
  current = rotateForward(current, settings.rotateQuadrants);
  if (Math.abs(settings.straighten) >= .01) {
    const aspect = settings.rotateQuadrants % 2 ? 1 / sourceAspect : sourceAspect;
    const angle = settings.straighten * Math.PI / 180; const dx = current.x - .5; const dy = current.y - .5;
    current = { x: Math.cos(angle) * dx - Math.sin(angle) * dy / aspect + .5, y: Math.sin(angle) * dx * aspect + Math.cos(angle) * dy + .5 };
  }
  return { x: (current.x - settings.cropLeft) / settings.cropWidth, y: (current.y - settings.cropTop) / settings.cropHeight };
}

export function displayToCanonical(point: MaskPoint, settings: BasicAdjustments, sourceAspect = 1, lens?: LensCorrections): MaskPoint {
  let current = { x: point.x * settings.cropWidth + settings.cropLeft, y: point.y * settings.cropHeight + settings.cropTop };
  if (Math.abs(settings.straighten) >= .01) {
    const aspect = settings.rotateQuadrants % 2 ? 1 / sourceAspect : sourceAspect;
    const angle = -settings.straighten * Math.PI / 180; const dx = current.x - .5; const dy = current.y - .5;
    current = { x: Math.cos(angle) * dx - Math.sin(angle) * dy / aspect + .5, y: Math.sin(angle) * dx * aspect + Math.cos(angle) * dy + .5 };
  }
  current = rotateBack(current, settings.rotateQuadrants);
  current = { x: settings.horizontalFlip ? 1 - current.x : current.x, y: settings.verticalFlip ? 1 - current.y : current.y };
  current = opticalSource(current, lens, sourceAspect);
  return { x: Math.max(0, Math.min(1, current.x)), y: Math.max(0, Math.min(1, current.y)) };
}

export function outputAspect(width: number, height: number, settings: BasicAdjustments) {
  const rotated = settings.rotateQuadrants % 2 !== 0;
  const orientedWidth = rotated ? height : width; const orientedHeight = rotated ? width : height;
  return (orientedWidth * settings.cropWidth) / Math.max(1, orientedHeight * settings.cropHeight);
}
