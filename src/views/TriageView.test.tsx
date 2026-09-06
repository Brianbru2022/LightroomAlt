import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { demoAssets } from "../lib/demo";
import { TriageView } from "./TriageView";

describe("Triage adjustments", () => {
  it("exposes protected range controls in the main viewer and saves a candidate", async () => {
    const auto = vi.fn().mockResolvedValue({ exposure: 0, lightBalance: 0, dynamicRange: 100, colourBoost: 9 });
    const apply = vi.fn().mockResolvedValue({ id: "adjusted-one", kind: "adjusted", createdAt: new Date().toISOString(), state: "candidate", imageUrl: demoAssets[0].previewUrl, isPreferred: false });
    const saved = vi.fn();
    render(<TriageView assets={demoAssets} total={demoAssets.length} hasMore={false} loading={false} onLoadMore={vi.fn()} selected={demoAssets[0]} onSelect={vi.fn()} onDecision={vi.fn()} onWorkshop={vi.fn()} onMap={vi.fn()} onTags={vi.fn()} onAutoAdjustments={auto} onApplyAdjustments={apply} onAdjustmentSaved={saved} />);

    expect(screen.getByRole("heading", { name: "Adjust" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Maximise range/ }));
    await waitFor(() => expect(screen.getByLabelText("Dynamic range")).toHaveValue("100"));
    expect(screen.getByLabelText("Colour boost")).toHaveValue("9");

    fireEvent.click(screen.getByRole("button", { name: "Save as candidate" }));
    await waitFor(() => expect(apply).toHaveBeenCalledWith(demoAssets[0], { exposure: 0, lightBalance: 0, dynamicRange: 100, colourBoost: 9 }));
    expect(saved).toHaveBeenCalledOnce();
  });
});
