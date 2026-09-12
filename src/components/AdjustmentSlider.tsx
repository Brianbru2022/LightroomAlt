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
  defaultValue?: number;
};

export function AdjustmentSlider({ label, value, min, max, step, suffix = "", low, high, onChange, defaultValue = 0 }: Props) {
  const shown = `${value > 0 ? "+" : ""}${Number.isInteger(value) ? value : value.toFixed(2)}${suffix}`;
  return (
    <label className="adjustment-slider">
      <span><strong>{label}</strong><output>{shown}</output></span>
      <input aria-label={label} type="range" min={min} max={max} step={step} value={value} onChange={(event) => onChange(Number(event.target.value))} />
      <input className="adjustment-number" aria-label={`${label} value`} type="number" min={min} max={max} step={step} value={value} onChange={(event) => { const next = Number(event.target.value); if (Number.isFinite(next)) onChange(Math.max(min, Math.min(max, next))); }} />
      {value !== defaultValue ? <button className="adjustment-reset" type="button" aria-label={`Reset ${label}`} onClick={() => onChange(defaultValue)}>Reset</button> : null}
      {low && high ? <small><span>{low}</span><span>{high}</span></small> : null}
    </label>
  );
}
