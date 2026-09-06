import { useEffect, useRef, useState } from "react";

export function Histogram({ src }: { src: string }) {
  const [bins, setBins] = useState<number[]>([]);
  const sourceRef = useRef(src); sourceRef.current = src;
  useEffect(() => {
    const image = new Image(); image.crossOrigin = "anonymous";
    image.onload = () => {
      if (sourceRef.current !== src) return;
      const canvas = document.createElement("canvas"); const scale = Math.min(1, 256 / Math.max(image.width, image.height));
      canvas.width = Math.max(1, Math.round(image.width * scale)); canvas.height = Math.max(1, Math.round(image.height * scale));
      const context = canvas.getContext("2d", { willReadFrequently: true }); if (!context) return;
      context.drawImage(image, 0, 0, canvas.width, canvas.height); const next = Array.from({ length: 64 }, () => 0);
      for (let offset = 0, data = context.getImageData(0, 0, canvas.width, canvas.height).data; offset < data.length; offset += 4) next[Math.min(63, Math.floor((data[offset] * .2126 + data[offset + 1] * .7152 + data[offset + 2] * .0722) / 4))]++;
      const maximum = Math.max(...next, 1); setBins(next.map((value) => value / maximum));
    }; image.src = src;
  }, [src]);
  const points = bins.map((value, index) => `${index * (160 / 63)},${48 - value * 46}`).join(" ");
  return <section className="histogram" aria-label="Luminance histogram"><span>Histogram</span><svg viewBox="0 0 160 50" role="img" aria-label="Brightness distribution"><polyline points={points} /></svg></section>;
}
