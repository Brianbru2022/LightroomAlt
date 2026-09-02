import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { App } from "./App";

describe("Keepframe application shell", () => {
  afterEach(() => vi.restoreAllMocks());
  it("renders the complete demo catalogue and navigates to triage", async () => {
    render(<App />);
    expect(await screen.findByRole("heading", { name: "Photographs" })).toBeInTheDocument();
    expect(await screen.findByText("DSC_1842.NEF")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Triage/ }));
    expect(await screen.findByText("Original protected")).toBeInTheDocument();
  });

  it("filters discarded photographs without deleting them", async () => {
    render(<App />);
    await screen.findByRole("heading", { name: "Photographs" });
    fireEvent.click(screen.getByRole("button", { name: "Discard", current: false }));
    await waitFor(() => expect(screen.getByText("DSC_0904.ARW")).toBeInTheDocument());
    expect(screen.queryByText("DSC_1842.NEF")).not.toBeInTheDocument();
  });

  it("uses X and M to mark the selected library photograph without hijacking text input", async () => {
    render(<App />);
    await screen.findByRole("heading", { name: "Photographs" });
    await screen.findByRole("button", { name: /DSC_1842.NEF/ });

    fireEvent.keyDown(window, { key: "x" });
    await waitFor(() => expect(screen.getByRole("button", { name: "DSC_1842.NEF, discard" })).toBeInTheDocument());

    const search = screen.getByRole("textbox", { name: "Search photographs" });
    fireEvent.keyDown(search, { key: "m" });
    expect(screen.getByRole("button", { name: "DSC_1842.NEF, discard" })).toBeInTheDocument();

    fireEvent.blur(search);
    fireEvent.keyDown(window, { key: "m" });
    await waitFor(() => expect(screen.getByRole("button", { name: "DSC_1842.NEF, keep" })).toBeInTheDocument());
  });

  it("moves the selected library photograph with the arrow keys", async () => {
    render(<App />);
    await screen.findByRole("heading", { name: "Photographs" });
    const first = await screen.findByRole("button", { name: /DSC_1842.NEF/ });
    const second = screen.getByRole("button", { name: /IMG_7721.CR3/ });

    expect(first).toHaveAttribute("aria-current", "true");
    fireEvent.keyDown(window, { key: "ArrowRight" });
    await waitFor(() => expect(second).toHaveAttribute("aria-current", "true"));
    expect(document.activeElement).toBe(second);

    fireEvent.keyDown(window, { key: "ArrowLeft" });
    await waitFor(() => expect(first).toHaveAttribute("aria-current", "true"));

    const search = screen.getByRole("textbox", { name: "Search photographs" });
    fireEvent.keyDown(search, { key: "ArrowRight" });
    expect(first).toHaveAttribute("aria-current", "true");
  });

  it("auto-advances triage decisions and undoes them with Ctrl+Z", async () => {
    render(<App />);
    await screen.findByRole("heading", { name: "Photographs" });
    fireEvent.click(screen.getByRole("button", { name: /Triage/ }));
    expect(await screen.findByRole("heading", { name: "DSC_1842.NEF" })).toBeInTheDocument();

    fireEvent.keyDown(window, { key: "ArrowLeft" });
    expect(await screen.findByRole("heading", { name: "IMG_7721.CR3" })).toBeInTheDocument();
    expect(await screen.findByText(/DSC_1842.NEF marked Discard/)).toBeInTheDocument();

    fireEvent.keyDown(window, { key: "z", ctrlKey: true });
    await waitFor(() => expect(screen.getByText("Latest catalogue change undone.")).toBeInTheDocument());
    fireEvent.click(screen.getByRole("button", { name: "Library" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "DSC_1842.NEF, keep" })).toBeInTheDocument());
  });

  it("supports the advertised Triage navigation and view shortcuts", async () => {
    render(<App />);
    await screen.findByRole("heading", { name: "Photographs" });
    fireEvent.click(screen.getByRole("button", { name: /Triage/ }));
    expect(await screen.findByRole("heading", { name: "DSC_1842.NEF" })).toBeInTheDocument();

    fireEvent.keyDown(window, { key: "ArrowRight", shiftKey: true });
    expect(await screen.findByRole("heading", { name: "IMG_7721.CR3" })).toBeInTheDocument();
    fireEvent.keyDown(window, { key: "ArrowLeft", shiftKey: true });
    expect(await screen.findByRole("heading", { name: "DSC_1842.NEF" })).toBeInTheDocument();

    fireEvent.keyDown(window, { key: "t" });
    expect(await screen.findByRole("dialog", { name: /Tags for DSC_1842.NEF/ })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Close tag editor" }));
    fireEvent.keyDown(window, { key: "e" });
    expect(await screen.findByRole("heading", { name: "AI Workshop" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Triage/ }));
    fireEvent.keyDown(window, { key: "m" });
    expect(await screen.findByRole("heading", { name: /places/ })).toBeInTheDocument();
  });

  it("deletes all photographs shown in the discard folder after confirmation", async () => {
    vi.spyOn(window, "confirm").mockReturnValue(true);
    render(<App />);
    await screen.findByRole("heading", { name: "Photographs" });
    fireEvent.click(screen.getByRole("button", { name: "Discard", current: false }));
    const deleteAll = await screen.findByRole("button", { name: /Delete all 1 discarded photograph/ });
    fireEvent.click(deleteAll);
    await waitFor(() => expect(screen.getByText("No photographs match")).toBeInTheDocument());
    expect(window.confirm).toHaveBeenCalledWith(expect.stringContaining("Windows Recycle Bin"));
  });
});
