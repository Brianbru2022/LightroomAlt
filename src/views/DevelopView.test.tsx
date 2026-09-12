import { fireEvent, render, screen, waitFor } from "@testing-library/react";
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
});
