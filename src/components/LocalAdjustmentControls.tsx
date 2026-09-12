import type { LocalAdjustments } from "../types";
import { AdjustmentSlider } from "./AdjustmentSlider";

type Props = { value: LocalAdjustments; onChange: (key: keyof LocalAdjustments, value: number) => void };
const controls: [keyof LocalAdjustments, string][] = [
  ["exposure", "Exposure"], ["contrast", "Contrast"], ["highlights", "Highlights"], ["shadows", "Shadows"],
  ["whites", "Whites"], ["blacks", "Blacks"], ["lightBalance", "Temperature"], ["tint", "Tint"],
  ["saturation", "Saturation"], ["texture", "Texture"], ["clarity", "Clarity"], ["dehaze", "Dehaze"],
];

export function LocalAdjustmentControls({ value, onChange }: Props) {
  return <div className="local-adjustment-grid">{controls.map(([key, label]) => <AdjustmentSlider key={key} label={label} value={value[key]} min={key === "exposure" ? -3 : -100} max={key === "exposure" ? 3 : 100} step={key === "exposure" ? .05 : 1} suffix={key === "exposure" ? " EV" : ""} onChange={(next) => onChange(key, next)} />)}</div>;
}
