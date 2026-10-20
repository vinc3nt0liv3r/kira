import type {
  Dimension,
  EventsResponse,
  Meta,
  Overview,
  TopResponse,
} from "./types";

const BASE = "/api/v1";

type Params = Record<string, string | number | undefined | null>;

async function get<T>(path: string, params: Params = {}, signal?: AbortSignal): Promise<T> {
  const qs = new URLSearchParams();
  for (const [key, value] of Object.entries(params)) {
    if (value === undefined || value === null || value === "") continue;
    qs.set(key, String(value));
  }
  const query = qs.size > 0 ? `?${qs}` : "";
  const res = await fetch(`${BASE}${path}${query}`, {
    signal,
    headers: { Accept: "application/json" },
  });
  if (!res.ok) {
    let message = `Request failed (${res.status})`;
    try {
      const body = (await res.json()) as { error?: string };
      if (body.error) message = body.error;
    } catch {
      // Keep the status-based message when the body is not JSON.
    }
    throw new Error(message);
  }
  return (await res.json()) as T;
}

export const api = {
  meta: (tenant?: string, signal?: AbortSignal) =>
    get<Meta>("/meta", { tenant }, signal),
  overview: (tenant: string, from: string, signal?: AbortSignal) =>
    get<Overview>("/overview", { tenant, from }, signal),
  top: (tenant: string, from: string, dimension: Dimension, limit: number, signal?: AbortSignal) =>
    get<TopResponse>("/top", { tenant, from, dimension, limit }, signal),
  events: (
    tenant: string,
    from: string,
    opts: { q?: string; action?: string; group?: string; limit?: number },
    signal?: AbortSignal,
  ) => get<EventsResponse>("/events", { tenant, from, ...opts }, signal),
};
