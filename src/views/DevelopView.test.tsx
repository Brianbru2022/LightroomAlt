import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { demoAssets } from "../lib/demo";
import { DevelopView } from "./DevelopView";

describe("Develop workspace", () => {
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
});
