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

    act(() => result.current.request(demoAssets[0], { ...neutralAdjustments, exposure: 0.1 }));
    await act(async () => { await vi.advanceTimersByTimeAsync(16); });
    expect(renderPreview).toHaveBeenCalledTimes(1);

    act(() => {
      result.current.request(demoAssets[0], { ...neutralAdjustments, exposure: 0.2 });
      result.current.request(demoAssets[0], { ...neutralAdjustments, exposure: 0.3 });
      finishFirst("first.png");
    });
    await act(async () => { await Promise.resolve(); await vi.advanceTimersByTimeAsync(16); });

    expect(renderPreview).toHaveBeenCalledTimes(2);
    expect(renderPreview).toHaveBeenLastCalledWith(demoAssets[0], { ...neutralAdjustments, exposure: 0.3 });
    expect(rendered).toHaveBeenLastCalledWith("latest.png");
  });
});
