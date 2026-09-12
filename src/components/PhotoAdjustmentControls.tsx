import { FlipHorizontal, FlipVertical, RotateCcw, RotateCw } from "lucide-react";
import type { BasicAdjustments } from "../types";
import { AdjustmentSlider } from "./AdjustmentSlider";

type Props = {
  value: BasicAdjustments;
  onChange: (key: keyof BasicAdjustments, value: number) => void;
  onReplace?: (next: BasicAdjustments) => void;
  compact?: boolean;
};

export function PhotoAdjustmentControls({ value, onChange, onReplace, compact = false }: Props) {
  type NumericKey = Exclude<keyof BasicAdjustments, "horizontalFlip" | "verticalFlip">;
  const slider = (key: NumericKey, label: string, low?: string, high?: string, suffix = "") => (
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
      <details className="adjustment-group">
        <summary>Crop & orientation</summary>
        <div className="adjustment-group-grid">
          <div className="adjustment-orientation-buttons"><button type="button" aria-label="Rotate left" onClick={() => onChange("rotateQuadrants", (value.rotateQuadrants + 3) % 4)}><RotateCcw size={15} /> Rotate left</button><button type="button" aria-label="Rotate right" onClick={() => onChange("rotateQuadrants", (value.rotateQuadrants + 1) % 4)}><RotateCw size={15} /> Rotate right</button><button type="button" aria-pressed={value.horizontalFlip} onClick={() => onReplace?.({ ...value, horizontalFlip: !value.horizontalFlip })}><FlipHorizontal size={15} /> Flip horizontal</button><button type="button" aria-pressed={value.verticalFlip} onClick={() => onReplace?.({ ...value, verticalFlip: !value.verticalFlip })}><FlipVertical size={15} /> Flip vertical</button></div>
          <label className="crop-ratio">Aspect ratio<select value="" onChange={(event) => { const ratio = event.target.value === "original" ? null : Number(event.target.value); if (onReplace) { const width = ratio ? Math.min(1, ratio) : 1; const height = ratio ? Math.min(1, 1 / ratio) : 1; onReplace({ ...value, cropLeft: (1 - width) / 2, cropTop: (1 - height) / 2, cropWidth: width, cropHeight: height }); } event.currentTarget.value = ""; }}><option value="" disabled>Choose ratio…</option><option value="original">Original / free</option><option value="1">1:1</option><option value="1.333333">4:3</option><option value="0.75">3:4</option><option value="1.5">3:2</option><option value="0.666667">2:3</option><option value="1.777778">16:9</option><option value="0.5625">9:16</option></select></label>
          <AdjustmentSlider label="Straighten" value={value.straighten} min={-15} max={15} step={0.1} suffix="°" onChange={(next) => onChange("straighten", next)} />
          <AdjustmentSlider label="Crop left" value={value.cropLeft} min={0} max={0.95} step={0.01} onChange={(next) => onChange("cropLeft", Math.min(next, 1 - value.cropWidth))} />
          <AdjustmentSlider label="Crop top" value={value.cropTop} min={0} max={0.95} step={0.01} onChange={(next) => onChange("cropTop", Math.min(next, 1 - value.cropHeight))} />
          <AdjustmentSlider label="Crop width" value={value.cropWidth} min={0.05} max={1} step={0.01} defaultValue={1} onChange={(next) => onChange("cropWidth", Math.min(next, 1 - value.cropLeft))} />
          <AdjustmentSlider label="Crop height" value={value.cropHeight} min={0.05} max={1} step={0.01} defaultValue={1} onChange={(next) => onChange("cropHeight", Math.min(next, 1 - value.cropTop))} />
          {onReplace ? <button type="button" className="reset-geometry-button" onClick={() => onReplace({ ...value, cropLeft: 0, cropTop: 0, cropWidth: 1, cropHeight: 1, rotateQuadrants: 0, straighten: 0, horizontalFlip: false, verticalFlip: false })}>Reset crop & orientation</button> : null}
        </div>
      </details>
    </div>
  );
}
