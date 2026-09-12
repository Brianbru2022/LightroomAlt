import { act, renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { demoAssets } from "../lib/demo";
import { neutralAdjustments } from "../types";
import { useAdjustmentPreview } from "./useAdjustmentPreview";

describe("useAdjustmentPreview", () => {
  afterEach(() => vi.useRealTimers());

  it("renders during a drag and coalesces queued positions to the latest value", async () => {
    vi.useFakeTimers();
    let finishFirst!: (url: string) => void;
    const renderPreview = vi.fn()
      .mockReturnValueOnce(new Promise<string>((resolve) => { finishFirst = resolve; }))
      .mockResolvedValueOnce("latest.png");
    const rendered = vi.fn();
    const { result } = renderHook(() => useAdjustmentPreview(renderPreview, rendered));

    act(() => result.current.request(demoAssets[0], { schemaVersion: 2, settings: { ...neutralAdjustments, exposure: 0.1 }, masks: [] }));
    await act(async () => { await vi.advanceTimersByTimeAsync(16); });
    expect(renderPreview).toHaveBeenCalledTimes(1);

    act(() => {
      result.current.request(demoAssets[0], { schemaVersion: 2, settings: { ...neutralAdjustments, exposure: 0.2 }, masks: [] });
      result.current.request(demoAssets[0], { schemaVersion: 2, settings: { ...neutralAdjustments, exposure: 0.3 }, masks: [] });
      finishFirst("first.png");
    });
    await act(async () => { await Promise.resolve(); await vi.advanceTimersByTimeAsync(16); });

    expect(renderPreview).toHaveBeenCalledTimes(2);
    expect(renderPreview).toHaveBeenLastCalledWith(demoAssets[0], { schemaVersion: 2, settings: { ...neutralAdjustments, exposure: 0.3 }, masks: [] });
    expect(rendered).toHaveBeenLastCalledWith("latest.png");
  });

  it("drops an in-flight result after an asset switch cancellation", async () => {
    vi.useFakeTimers(); let finish!: (url: string) => void;
    const rendered = vi.fn(); const { result } = renderHook(() => useAdjustmentPreview(() => new Promise<string>((resolve) => { finish = resolve; }), rendered));
    act(() => result.current.request(demoAssets[0], neutralAdjustments));
    await act(async () => { await vi.advanceTimersByTimeAsync(16); });
    act(() => { result.current.cancel(); finish("stale.png"); });
    await act(async () => { await Promise.resolve(); });
    expect(rendered).not.toHaveBeenCalled();
  });
});
