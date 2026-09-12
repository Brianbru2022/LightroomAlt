import type { MapBounds } from "../types";

export type TileProvider = { url: string; attribution: string; maxZoom: number; label: string };
export type PlaceResult = { label: string; latitude: number; longitude: number };

const developmentTiles: TileProvider = {
  url: "https://{s}.tile.openstreetmap.org/{z}/{x}/{y}.png",
  attribution: "&copy; <a href=\"https://www.openstreetmap.org/copyright\">OpenStreetMap</a> contributors",
  maxZoom: 19,
  label: "OpenStreetMap development tiles",
};
const env = (import.meta as unknown as { env?: Record<string, string | undefined> }).env ?? {};

const validTemplate = (value: string, required: string[]) => {
  try {
    const url = new URL(value.replaceAll("{s}", "a").replaceAll("{z}", "0").replaceAll("{x}", "0").replaceAll("{y}", "0").replaceAll("{query}", "test").replaceAll("{app}", "keepframe"));
    return (url.protocol === "https:" || (url.protocol === "http:" && ["localhost", "127.0.0.1", "[::1]"].includes(url.hostname))) && !url.username && !url.password && required.every((part) => value.includes(part));
  } catch { return false; }
};

export const isValidTileProvider = (url: string, attribution: string, maxZoom: number) => validTemplate(url, ["{z}", "{x}", "{y}"]) && Boolean(attribution.trim()) && Number.isInteger(maxZoom) && maxZoom >= 1 && maxZoom <= 22;
export const tileProvider = (): TileProvider => {
  const url = env.VITE_KEEPFRAME_TILE_URL?.trim();
  const attribution = env.VITE_KEEPFRAME_TILE_ATTRIBUTION?.trim();
  const maxZoom = Number(env.VITE_KEEPFRAME_TILE_MAX_ZOOM ?? developmentTiles.maxZoom);
  if (!url) return developmentTiles;
  if (!attribution || !isValidTileProvider(url, attribution, maxZoom)) return developmentTiles;
  return { url, attribution, maxZoom, label: "Configured map tiles" };
};

const geocoderUrl = () => env.VITE_KEEPFRAME_GEOCODER_URL?.trim();
const geocoderApplication = () => env.VITE_KEEPFRAME_GEOCODER_APPLICATION?.trim();
export const geocoderAvailable = () => {
  const url = geocoderUrl();
  return Boolean(url && geocoderApplication() && validTemplate(url, ["{query}", "{app}"]));
};

const cache = new Map<string, PlaceResult[]>();
export async function searchPlaces(query: string, signal?: AbortSignal): Promise<PlaceResult[]> {
  const text = query.trim();
  if (text.length < 2) return [];
  if (!geocoderAvailable()) throw new Error("Location search is not configured. Map browsing remains available.");
  const key = text.toLocaleLowerCase();
  const prior = cache.get(key); if (prior) return prior;
  const url = geocoderUrl()!
    .replaceAll("{query}", encodeURIComponent(text))
    .replaceAll("{app}", encodeURIComponent(geocoderApplication()!));
  const response = await fetch(url, { signal, headers: { Accept: "application/json" } });
  if (!response.ok) throw new Error("Location search is temporarily unavailable.");
  const payload: unknown = await response.json();
  const values = Array.isArray(payload) ? payload : payload && typeof payload === "object" && Array.isArray((payload as { results?: unknown[] }).results) ? (payload as { results: unknown[] }).results : [];
  const results = values.flatMap((value): PlaceResult[] => {
    if (!value || typeof value !== "object") return [];
    const item = value as { display_name?: unknown; label?: unknown; lat?: unknown; lon?: unknown; latitude?: unknown; longitude?: unknown };
    const latitude = Number(item.latitude ?? item.lat); const longitude = Number(item.longitude ?? item.lon);
    const label = typeof item.label === "string" ? item.label : typeof item.display_name === "string" ? item.display_name : "";
    return label && Number.isFinite(latitude) && latitude >= -90 && latitude <= 90 && Number.isFinite(longitude) && longitude >= -180 && longitude <= 180 ? [{ label, latitude, longitude }] : [];
  }).slice(0, 6);
  cache.set(key, results);
  return results;
}

export const boundsFromLeaflet = (bounds: { getSouth(): number; getWest(): number; getNorth(): number; getEast(): number }): MapBounds => ({ south: bounds.getSouth(), west: bounds.getWest(), north: bounds.getNorth(), east: bounds.getEast() });
