import type {
  Dimension,
  EventsResponse,
  Meta,
  Overview,
  RetentionPreview,
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
    opts: { q?: string; action?: string; group?: string; user?: string; category?: string; device?: string; limit?: number },
    signal?: AbortSignal,
  ) => get<EventsResponse>("/events", { tenant, from, ...opts }, signal),
  exportEvents: (filters: {
    tenant: string;
    from: string;
    q?: string;
    action?: string;
    group?: string;
    user?: string;
    category?: string;
    device?: string;
  }) => {
    const qs = new URLSearchParams();
    for (const [key, value] of Object.entries(filters)) {
      if (value === undefined || value === null || value === "") continue;
      qs.set(key, String(value));
    }
    window.open(`${BASE}/events/export?${qs.toString()}`, "_blank");
  },
  retention: (signal?: AbortSignal) =>
    get<RetentionPreview>("/retention", {}, signal),
  updateRetention: async (retention_hours: number, token: string): Promise<{ retention_hours: number; message: string }> => {
    const res = await fetch(`${BASE}/retention`, {
      method: "PUT",
      headers: {
        Accept: "application/json",
        "Content-Type": "application/json",
        Authorization: `Bearer ${token}`,
      },
      body: JSON.stringify({ retention_hours }),
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
    return (await res.json()) as { retention_hours: number; message: string };
  },
};
