import { describe, expect, it, vi } from "vitest";
import { isValidTileProvider, searchPlaces, tileProvider } from "./mapServices";

describe("map services", () => {
  it("uses the attribution-bearing development tile provider when no production provider is configured", () => {
    expect(tileProvider().url).toContain("{z}/{x}/{y}");
    expect(tileProvider().attribution).toContain("OpenStreetMap");
  });

  it("requires an attributed safe tile template", () => {
    expect(isValidTileProvider("https://tiles.example/{z}/{x}/{y}.png", "© Example", 19)).toBe(true);
    expect(isValidTileProvider("https://tiles.example/{z}/{x}/tile.png", "© Example", 19)).toBe(false);
    expect(isValidTileProvider("http://tiles.example/{z}/{x}/{y}.png", "© Example", 19)).toBe(false);
    expect(isValidTileProvider("https://user:secret@tiles.example/{z}/{x}/{y}.png", "© Example", 19)).toBe(false);
  });

  it("does not make location search requests when a provider is not configured", async () => {
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);
    await expect(searchPlaces("Stirling")).rejects.toThrow("not configured");
    expect(fetchMock).not.toHaveBeenCalled();
  });
});
