import { useCallback, useEffect, useRef } from "react";
import type { Asset } from "../types";

type PendingPreview<T> = { asset: Asset; value: T; generation: number };

export function useAdjustmentPreview<T>(
  render: (asset: Asset, value: T) => Promise<string>,
  onRendered: (url: string) => void,
  onError?: (error: unknown) => void,
) {
  const renderRef = useRef(render);
  const renderedRef = useRef(onRendered);
  const errorRef = useRef(onError);
  const timerRef = useRef<number | null>(null);
  const pendingRef = useRef<PendingPreview<T> | null>(null);
  const inFlightRef = useRef(0);
  const generationRef = useRef(0);
  renderRef.current = render;
  renderedRef.current = onRendered;
  errorRef.current = onError;

  const runNext = useCallback(() => {
    timerRef.current = null;
    if (inFlightRef.current >= 2 || !pendingRef.current) return;
    const pending = pendingRef.current;
    pendingRef.current = null;
    const generation = pending.generation;
    inFlightRef.current += 1;
    void renderRef.current(pending.asset, pending.value)
      .then((url) => {
        if (generationRef.current === generation) renderedRef.current(url);
      })
      .catch((error) => {
        if (generationRef.current === generation) errorRef.current?.(error);
      })
      .finally(() => {
        inFlightRef.current -= 1;
        if (pendingRef.current && timerRef.current === null) {
          timerRef.current = window.setTimeout(runNext, 16);
        }
      });
  }, []);

  const request = useCallback((asset: Asset, value: T) => {
    pendingRef.current = { asset, value, generation: ++generationRef.current };
    if (timerRef.current === null) {
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
