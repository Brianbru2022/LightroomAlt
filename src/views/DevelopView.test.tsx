import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { demoAssets } from "../lib/demo";
import { api } from "../lib/bridge";
import type { IntelligentMaskCategory, IntelligentMaskHealth, IntelligentMaskProposal } from "../types";
import { DevelopView } from "./DevelopView";

const intelligentHealth: IntelligentMaskHealth = { available: true, installed: true, runtimeAvailable: true, loaded: false, busy: false, provider: "Mock segmentation provider", providerVersion: "1", model: "mock/segmentation", modelRevision: "revision", licence: "Apache-2.0", source: "local-test", approximateBytes: 12, storagePath: "D:\\AI Models\\test", executionProvider: "CPU", detail: "Ready" };
const intelligentProposal = (category: IntelligentMaskCategory): IntelligentMaskProposal => ({ requestId: 1, category, confidence: .88, coverageFraction: .35, elapsedMs: 17, timings: { loadMs: 1, preprocessMs: 2, inferenceMs: 10, postprocessMs: 4 }, mask: { id: `mask-${category}`, name: category[0].toUpperCase() + category.slice(1), enabled: true, inverted: false, opacity: 1, feather: 0, geometry: { kind: "semantic", width: 16, height: 16, coveragePng: "iVBORw0KGgo=", checksum: "a".repeat(64), provenance: { provider: "Mock segmentation provider", providerVersion: "1", model: "mock/segmentation", modelRevision: "revision", modelSha256: "b".repeat(64), category, executionProvider: "CPU" }, refinements: [] }, adjustments: { exposure: 0, contrast: 0, highlights: 0, shadows: 0, whites: 0, blacks: 0, lightBalance: 0, tint: 0, saturation: 0, clarity: 0, dehaze: 0, texture: 0 } } });

afterEach(() => vi.restoreAllMocks());

describe("Develop workspace", () => {
  it("keeps manual masks available when the intelligent model is unavailable", async () => {
    const selected = { ...demoAssets[0], id: "model-unavailable", hasEdits: false };
    render(<DevelopView assets={[selected]} selected={selected} onSelect={vi.fn()} onExport={async () => undefined} onRecipeSaved={vi.fn()} />);
    expect(await screen.findByText(/Intelligent masking model not installed/)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Subject" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Linear" })).not.toBeDisabled();
  });
  it("groups an adjustment, exposes before/after and copies photographic settings only", async () => {
    const saved = vi.fn();
    const selected = { ...demoAssets[0], hasEdits: false };
    render(<DevelopView assets={[selected, demoAssets[1]]} selected={selected} onSelect={vi.fn()} onExport={async () => undefined} onRecipeSaved={saved} />);
    await screen.findByText("Develop");
    fireEvent.change(screen.getByRole("slider", { name: "Exposure" }), { target: { value: "1" } });
    await waitFor(() => expect(saved).toHaveBeenCalled());
    expect(screen.getByText("Edited")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /copy settings/i }));
    expect(screen.getByRole("button", { name: /paste settings/i })).not.toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: /original/i }));
    expect(screen.getByAltText(selected.filename).getAttribute("src")).toContain(selected.previewUrl);
  });

  it("keeps Auto and preset previews out of persistence until explicit Apply", async () => {
    const saved = vi.fn(); const selected = { ...demoAssets[0], hasEdits: false };
    render(<DevelopView assets={[selected]} selected={selected} onSelect={vi.fn()} onExport={async () => undefined} onRecipeSaved={saved} />);
    await screen.findByText("Analyse photo");
    fireEvent.click(screen.getByRole("button", { name: /analyse photo/i }));
    await screen.findByRole("button", { name: "Apply" });
    expect(saved).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    fireEvent.click(screen.getByRole("button", { name: "Warm" }));
    expect(await screen.findByText("Warm preview")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(saved).toHaveBeenCalled());
  });

  it("creates, edits, duplicates, and explicitly copies first-class masks", async () => {
    const saved = vi.fn(); const selected = { ...demoAssets[1], hasEdits: false };
    render(<DevelopView assets={[selected]} selected={selected} onSelect={vi.fn()} onExport={async () => undefined} onRecipeSaved={saved} />);
    const panel = await screen.findByRole("region", { name: "Masks" });
    fireEvent.click(within(panel).getByRole("button", { name: "Linear" }));
    expect(within(panel).getByDisplayValue("Linear gradient 1")).toBeTruthy();
    fireEvent.change(within(panel).getByLabelText("Mask name"), { target: { value: "Sky" } });
    fireEvent.click(within(panel).getByRole("button", { name: "Invert" }));
    fireEvent.click(within(panel).getByRole("button", { name: "Duplicate" }));
    expect(within(panel).getByDisplayValue("Sky copy")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    expect(within(panel).queryByDisplayValue("Sky copy")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Redo" }));
    expect(within(panel).getByDisplayValue("Sky copy")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /copy with masks/i }));
    expect(screen.getByRole("button", { name: /paste settings/i })).not.toBeDisabled();
    await waitFor(() => expect(saved).toHaveBeenCalled());
  });

  it("keeps intelligent predictions out of persistence until Accept and makes acceptance undoable", async () => {
    const saved = vi.fn(); const selected = { ...demoAssets[0], id: "intelligent-accept", hasEdits: false };
    vi.spyOn(api, "intelligentMaskHealth").mockResolvedValue(intelligentHealth);
    vi.spyOn(api, "proposeIntelligentMask").mockImplementation(async (_assetId, category) => intelligentProposal(category));
    render(<DevelopView assets={[selected]} selected={selected} onSelect={vi.fn()} onExport={async () => undefined} onRecipeSaved={saved} />);
    fireEvent.click(await screen.findByRole("button", { name: "Subject" }));
    expect(await screen.findByRole("region", { name: "Intelligent mask prediction" })).toBeTruthy();
    expect(saved).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.queryByText("AI subject")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "People" }));
    await screen.findByText("People prediction");
    fireEvent.click(screen.getByRole("button", { name: "Accept" }));
    expect(await screen.findByText("AI people")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Add Brush" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Subtract Brush" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    expect(screen.queryByText("AI people")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Redo" }));
    expect(await screen.findByText("AI people")).toBeTruthy();
    await waitFor(() => expect(saved).toHaveBeenCalled());
  });

  it("logically cancels inference and discards stale completion after an asset switch", async () => {
    const first = { ...demoAssets[0], id: "stale-first", hasEdits: false }; const second = { ...demoAssets[1], id: "stale-second", hasEdits: false }; let resolve: ((proposal: IntelligentMaskProposal) => void) | undefined;
    vi.spyOn(api, "intelligentMaskHealth").mockResolvedValue(intelligentHealth);
    vi.spyOn(api, "cancelIntelligentMask").mockResolvedValue(undefined);
    vi.spyOn(api, "proposeIntelligentMask").mockImplementation(() => new Promise((done) => { resolve = done; }));
    const view = render(<DevelopView assets={[first, second]} selected={first} onSelect={vi.fn()} onExport={async () => undefined} onRecipeSaved={vi.fn()} />);
    fireEvent.click(await screen.findByRole("button", { name: "Sky" }));
    expect(await screen.findByText("Analysing Sky…")).toBeTruthy();
    view.rerender(<DevelopView assets={[first, second]} selected={second} onSelect={vi.fn()} onExport={async () => undefined} onRecipeSaved={vi.fn()} />);
    resolve?.(intelligentProposal("sky"));
    await Promise.resolve();
    expect(screen.queryByText("Sky prediction")).toBeNull();
    expect(api.cancelIntelligentMask).toHaveBeenCalled();
  });
});
