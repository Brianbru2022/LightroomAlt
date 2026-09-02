import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { demoAssets } from "../lib/demo";
import { LibraryView } from "./LibraryView";

describe("virtualised library", () => {
  it("renders only the visible window from a large loaded page", () => {
    const assets = Array.from({ length: 1_000 }, (_, index) => ({
      ...demoAssets[0],
      id: `large-${index}`,
      filename: `photo-${index}.jpg`,
      capturedAt: `2026-08-${String(index % 28 + 1).padStart(2, "0")}T10:00:00Z`,
    }));
    render(<LibraryView assets={assets} total={10_000} loading={false} hasMore onLoadMore={vi.fn()} selected={assets[0]} onSelect={vi.fn()} onOpen={vi.fn()} />);

    const renderedCards = screen.getAllByRole("button", { name: /photo-\d+\.jpg/ });
    expect(renderedCards.length).toBeGreaterThan(0);
    expect(renderedCards.length).toBeLessThan(100);
    expect(screen.getAllByText(/1000 loaded/).length).toBeGreaterThan(0);
  });
});
