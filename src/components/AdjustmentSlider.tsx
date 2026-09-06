type Props = {
  label: string;
  value: number;
  min: number;
  max: number;
  step: number;
  suffix?: string;
  low?: string;
  high?: string;
  onChange: (value: number) => void;
};

export function AdjustmentSlider({ label, value, min, max, step, suffix = "", low, high, onChange }: Props) {
  const shown = `${value > 0 ? "+" : ""}${Number.isInteger(value) ? value : value.toFixed(2)}${suffix}`;
  return (
    <label className="adjustment-slider">
      <span><strong>{label}</strong><output>{shown}</output></span>
      <input aria-label={label} type="range" min={min} max={max} step={step} value={value} onChange={(event) => onChange(Number(event.target.value))} />
      {low && high ? <small><span>{low}</span><span>{high}</span></small> : null}
    </label>
  );
}
