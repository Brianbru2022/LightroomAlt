import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { demoAssets } from "../lib/demo";
import { TriageView } from "./TriageView";

describe("Triage adjustments", () => {
  it("exposes protected range controls in the main viewer and saves a candidate", async () => {
    const auto = vi.fn().mockResolvedValue({ exposure: 0, lightBalance: 0, dynamicRange: 100, colourBoost: 9 });
    const apply = vi.fn().mockResolvedValue({ id: "adjusted-one", kind: "adjusted", createdAt: new Date().toISOString(), state: "candidate", imageUrl: demoAssets[0].previewUrl, isPreferred: false });
    const saved = vi.fn();
    const preview = vi.fn().mockResolvedValueOnce("adjusted-preview.png").mockResolvedValueOnce("adjusted-preview-exposure.png");
    render(<TriageView assets={demoAssets} total={demoAssets.length} hasMore={false} loading={false} onLoadMore={vi.fn()} selected={demoAssets[0]} onSelect={vi.fn()} onDecision={vi.fn()} onWorkshop={vi.fn()} onMap={vi.fn()} onTags={vi.fn()} onAutoAdjustments={auto} onPreviewAdjustments={preview} onApplyAdjustments={apply} onAdjustmentSaved={saved} />);

    expect(screen.getByRole("heading", { name: "Adjust" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Maximise range/ }));
    await waitFor(() => expect(screen.getByLabelText("Dynamic range")).toHaveValue("100"));
    expect(screen.getByLabelText("Colour boost")).toHaveValue("9");
    expect(preview).toHaveBeenCalledWith(demoAssets[0], { exposure: 0, lightBalance: 0, dynamicRange: 100, colourBoost: 9 });

    const photograph = screen.getAllByAltText(demoAssets[0].filename)[0];
    expect(photograph).toHaveAttribute("src", "adjusted-preview.png");
    fireEvent.change(screen.getByRole("slider", { name: "Exposure" }), { target: { value: "0.5" } });
    expect(photograph).toHaveAttribute("src", "adjusted-preview.png");
    await waitFor(() => expect(preview).toHaveBeenLastCalledWith(demoAssets[0], { exposure: 0.5, lightBalance: 0, dynamicRange: 100, colourBoost: 9 }));
    await waitFor(() => expect(photograph).toHaveAttribute("src", "adjusted-preview-exposure.png"));

    fireEvent.click(screen.getByRole("button", { name: "Save as candidate" }));
    await waitFor(() => expect(apply).toHaveBeenCalledWith(demoAssets[0], { exposure: 0.5, lightBalance: 0, dynamicRange: 100, colourBoost: 9 }));
    expect(saved).toHaveBeenCalledOnce();
  });
});
