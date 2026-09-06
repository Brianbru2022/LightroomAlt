import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { demoAssets, makeRecipe, renderPrompts } from "../lib/demo";
import { neutralAdjustments, type BatchJob } from "../types";
import { WorkshopView } from "./WorkshopView";

describe("AI Workshop batch review", () => {
  it("edits an image-specific recipe and bulk-approves only reviewed jobs", async () => {
    const recipe = makeRecipe(demoAssets[0], "restoration");
    const job: BatchJob = {
      id: "job-reviewed",
      batchId: "batch-one",
      assetId: demoAssets[0].id,
      assetName: demoAssets[0].filename,
      state: "review_required",
      prompt: renderPrompts(recipe).local,
      recipe,
      attempts: [],
    };
    const save = vi.fn().mockResolvedValue(undefined);
    const approve = vi.fn().mockResolvedValue(undefined);
    const auto = vi.fn().mockResolvedValue({ ...neutralAdjustments, dynamicRange: 100, highlights: -30, shadows: 20, colourBoost: 9 });
    render(<WorkshopView asset={demoAssets[0]} assets={demoAssets} jobs={[job]} serviceHealth={{ localAiAvailable: true, serviceReachable: true, localAiBusy: false, localAiModel: "Qwen-Image-Edit", localAiDetail: "Ready", analysisModelInstalled: false, analysisAvailable: false, analysisDetail: "Controls-only fallback" }} onAnalyse={vi.fn().mockResolvedValue(recipe)} onPrompts={vi.fn().mockResolvedValue(renderPrompts(recipe))} onCopy={vi.fn()} onPrepare={vi.fn()} onExportExternal={vi.fn()} onImportReturned={vi.fn()} onLoadVersions={vi.fn().mockResolvedValue([])} onSetPreferred={vi.fn()} onReplace={vi.fn()} onAutoAdjustments={auto} onPreviewAdjustments={vi.fn().mockResolvedValue("adjusted-preview.png")} onApplyAdjustments={vi.fn()} onEnqueue={vi.fn()} onRunLocal={vi.fn()} onJob={vi.fn()} onSaveJobReview={save} onApproveJobs={approve} />);

    fireEvent.click(screen.getByRole("button", { name: "Maximise range" }));
    await waitFor(() => expect(screen.getAllByLabelText("Highlights")[0]).toHaveValue("-30"));
    expect(screen.getByRole("slider", { name: "Vibrance" })).toHaveValue("9");
    expect(auto).toHaveBeenCalledWith(demoAssets[0].id);

    fireEvent.click(screen.getByText("Review image-specific recipe"));
    const prompt = screen.getByLabelText("Final local prompt");
    fireEvent.change(prompt, { target: { value: "A distinct reviewed prompt for this photograph." } });
    fireEvent.click(screen.getByRole("button", { name: "Save this recipe" }));
    expect(save).toHaveBeenCalledWith("job-reviewed", expect.objectContaining({ assetId: demoAssets[0].id }), "A distinct reviewed prompt for this photograph.");

    fireEvent.click(screen.getByRole("button", { name: "Approve all reviewed" }));
    expect(approve).toHaveBeenCalledWith(["job-reviewed"]);
  });
});
