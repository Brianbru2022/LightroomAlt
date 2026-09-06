import type { BasicAdjustments } from "../types";
import { AdjustmentSlider } from "./AdjustmentSlider";

type Props = {
  value: BasicAdjustments;
  onChange: (key: keyof BasicAdjustments, value: number) => void;
  compact?: boolean;
};

export function PhotoAdjustmentControls({ value, onChange, compact = false }: Props) {
  const slider = (key: keyof BasicAdjustments, label: string, low?: string, high?: string, suffix = "") => (
    <AdjustmentSlider key={key} label={label} value={value[key]} min={-100} max={100} step={1} low={low} high={high} suffix={suffix} onChange={(next) => onChange(key, next)} />
  );
  return (
    <div className={`photo-adjustment-controls ${compact ? "compact" : ""}`}>
      <details className="adjustment-group" open>
        <summary>Tone</summary>
        <div className="adjustment-group-grid">
          <AdjustmentSlider label="Exposure" value={value.exposure} min={-3} max={3} step={0.05} suffix=" EV" onChange={(next) => onChange("exposure", next)} />
          {slider("contrast", "Contrast")}
          {slider("highlights", "Highlights")}
          {slider("shadows", "Shadows")}
          {slider("whites", "Whites")}
          {slider("blacks", "Blacks")}
        </div>
      </details>
      <details className="adjustment-group">
        <summary>White balance</summary>
        <div className="adjustment-group-grid">
          {slider("lightBalance", "Temperature", "Cool", "Warm")}
          {slider("tint", "Tint", "Green", "Magenta")}
        </div>
      </details>
      <details className="adjustment-group">
        <summary>Presence & colour</summary>
        <div className="adjustment-group-grid">
          {slider("texture", "Texture")}
          {slider("clarity", "Clarity")}
          {slider("dehaze", "Dehaze")}
          {slider("colourBoost", "Vibrance")}
          {slider("saturation", "Saturation")}
        </div>
      </details>
      <details className="adjustment-group">
        <summary>Tone curve</summary>
        <div className="adjustment-group-grid">
          {slider("curveHighlights", "Highlights")}
          {slider("curveLights", "Lights")}
          {slider("curveDarks", "Darks")}
          {slider("curveShadows", "Shadows")}
        </div>
      </details>
    </div>
  );
}
