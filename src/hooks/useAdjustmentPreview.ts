import { useCallback, useEffect, useRef } from "react";
import type { Asset, BasicAdjustments } from "../types";

type PendingPreview = { asset: Asset; adjustments: BasicAdjustments };

export function useAdjustmentPreview(
  render: (asset: Asset, adjustments: BasicAdjustments) => Promise<string>,
  onRendered: (url: string) => void,
  onError?: (error: unknown) => void,
) {
  const renderRef = useRef(render);
  const renderedRef = useRef(onRendered);
  const errorRef = useRef(onError);
  const timerRef = useRef<number | null>(null);
  const pendingRef = useRef<PendingPreview | null>(null);
  const inFlightRef = useRef(false);
  const generationRef = useRef(0);
  renderRef.current = render;
  renderedRef.current = onRendered;
  errorRef.current = onError;

  const runNext = useCallback(() => {
    timerRef.current = null;
    if (inFlightRef.current || !pendingRef.current) return;
    const pending = pendingRef.current;
    pendingRef.current = null;
    const generation = generationRef.current;
    inFlightRef.current = true;
    void renderRef.current(pending.asset, pending.adjustments)
      .then((url) => {
        if (generationRef.current === generation) renderedRef.current(url);
      })
      .catch((error) => {
        if (generationRef.current === generation) errorRef.current?.(error);
      })
      .finally(() => {
        inFlightRef.current = false;
        if (pendingRef.current && timerRef.current === null) {
          timerRef.current = window.setTimeout(runNext, 16);
        }
      });
  }, []);

  const request = useCallback((asset: Asset, adjustments: BasicAdjustments) => {
    pendingRef.current = { asset, adjustments };
    if (!inFlightRef.current && timerRef.current === null) {
      timerRef.current = window.setTimeout(runNext, 16);
    }
  }, [runNext]);

  const cancel = useCallback(() => {
    generationRef.current += 1;
    pendingRef.current = null;
    if (timerRef.current !== null) window.clearTimeout(timerRef.current);
    timerRef.current = null;
  }, []);

  useEffect(() => cancel, [cancel]);
  return { request, cancel };
}
