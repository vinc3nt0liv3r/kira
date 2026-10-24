import type {
  AlertInstance,
  AlertRuleResponse,
  CreateAlertRule,
  CreateCustomReport,
  CreateReportSchedule,
  CreateReportTemplate,
  CustomReportResponse,
  Dimension,
  EventsResponse,
  Meta,
  Overview,
  ReportResult,
  ReportScheduleResponse,
  ReportTemplateResponse,
  RetentionPreview,
  TestWebhookResult,
  TopResponse,
  UpdateAlertRule,
  UpdateReportSchedule,
  WebhookDelivery,
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

  // Alert rules
  listAlertRules: (signal?: AbortSignal) =>
    get<{ rules: AlertRuleResponse[] }>("/api/v1/alerts/rules", {}, signal),
  createAlertRule: async (payload: CreateAlertRule): Promise<AlertRuleResponse> => {
    const res = await fetch(`${BASE}/alerts/rules`, {
      method: "POST",
      headers: {
        Accept: "application/json",
        "Content-Type": "application/json",
      },
      body: JSON.stringify(payload),
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
    const data = await res.json() as { rule: AlertRuleResponse };
    return data.rule;
  },
  updateAlertRule: async (id: string, payload: UpdateAlertRule): Promise<AlertRuleResponse> => {
    const res = await fetch(`${BASE}/alerts/rules/${id}`, {
      method: "PUT",
      headers: {
        Accept: "application/json",
        "Content-Type": "application/json",
      },
      body: JSON.stringify(payload),
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
    const data = await res.json() as { rule: AlertRuleResponse };
    return data.rule;
  },
  deleteAlertRule: async (id: string): Promise<void> => {
    const res = await fetch(`${BASE}/alerts/rules/${id}`, {
      method: "DELETE",
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
  },
  listAlertInstances: (signal?: AbortSignal) =>
    get<{ instances: AlertInstance[] }>("/api/v1/alerts/instances", {}, signal),
  acknowledgeAlertInstance: async (id: string, acknowledged_by: string): Promise<void> => {
    const res = await fetch(`${BASE}/alerts/instances/${id}/acknowledge`, {
      method: "POST",
      headers: {
        Accept: "application/json",
        "Content-Type": "application/json",
      },
      body: JSON.stringify({ acknowledged_by }),
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
  },
  listWebhookDeliveries: (instanceId: string, signal?: AbortSignal) =>
    get<{ deliveries: WebhookDelivery[] }>(`/api/v1/alerts/instances/${instanceId}/webhook-deliveries`, {}, signal),
  testWebhook: async (url: string): Promise<TestWebhookResult> => {
    const res = await fetch(`${BASE}/webhook/test`, {
      method: "POST",
      headers: {
        Accept: "application/json",
        "Content-Type": "application/json",
      },
      body: JSON.stringify({ url }),
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
    return (await res.json()) as TestWebhookResult;
  },

  // Report schedules
  listReportSchedules: (signal?: AbortSignal) =>
    get<{ schedules: ReportScheduleResponse[] }>("/api/v1/reports/schedules", {}, signal),
  createReportSchedule: async (payload: CreateReportSchedule): Promise<ReportScheduleResponse> => {
    const res = await fetch(`${BASE}/reports/schedules`, {
      method: "POST",
      headers: {
        Accept: "application/json",
        "Content-Type": "application/json",
      },
      body: JSON.stringify(payload),
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
    const data = await res.json() as { schedule: ReportScheduleResponse };
    return data.schedule;
  },
  updateReportSchedule: async (id: string, payload: UpdateReportSchedule): Promise<ReportScheduleResponse> => {
    const res = await fetch(`${BASE}/reports/schedules/${id}`, {
      method: "PUT",
      headers: {
        Accept: "application/json",
        "Content-Type": "application/json",
      },
      body: JSON.stringify(payload),
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
    const data = await res.json() as { schedule: ReportScheduleResponse };
    return data.schedule;
  },
  deleteReportSchedule: async (id: string): Promise<void> => {
    const res = await fetch(`${BASE}/reports/schedules/${id}`, {
      method: "DELETE",
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
  },
  listReportResults: (scheduleId: string, signal?: AbortSignal) =>
    get<{ results: ReportResult[] }>(`/api/v1/reports/schedules/${scheduleId}/results`, {}, signal),
  generateReportNow: async (scheduleId: string): Promise<{ message: string }> => {
    const res = await fetch(`${BASE}/reports/schedules/${scheduleId}/generate`, {
      method: "POST",
      headers: {
        Accept: "application/json",
      },
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
    return (await res.json()) as { message: string };
  },
  downloadReportResult: async (scheduleId: string, resultId: string): Promise<void> => {
    const res = await fetch(`${BASE}/reports/schedules/${scheduleId}/results/${resultId}/download`, {
      method: "GET",
      headers: { Accept: "application/octet-stream" },
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
    const blob = await res.blob();
    const url = window.URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    const disposition = res.headers.get("Content-Disposition");
    const filename = disposition
      ?.match(/filename[^;=\n]*=((['"])(.*?)\2|[^\n;]*)/)
      ?.pop()?.replace(/['"]/g, "") || "report";
    a.download = filename;
    document.body.appendChild(a);
    a.click();
    window.URL.revokeObjectURL(url);
    document.body.removeChild(a);
  },
  downloadReportOnDemand: async (payload: {
    format?: string;
    filters?: Record<string, unknown>;
    tenant?: string;
  }): Promise<void> => {
    const qs = new URLSearchParams();
    if (payload.tenant) qs.set("tenant", payload.tenant);
    const res = await fetch(`${BASE}/reports/generate?${qs.toString()}`, {
      method: "POST",
      headers: {
        Accept: "application/octet-stream",
        "Content-Type": "application/json",
      },
      body: JSON.stringify({
        format: payload.format ?? "pdf",
        filters: payload.filters ?? {},
      }),
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
    const blob = await res.blob();
    const url = window.URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    const disposition = res.headers.get("Content-Disposition");
    const filename = disposition
      ?.match(/filename[^;=\n]*=((['"])(.*?)\2|[^\n;]*)/)
      ?.pop()?.replace(/['"]/g, "") || "report";
    a.download = filename;
    document.body.appendChild(a);
    a.click();
    window.URL.revokeObjectURL(url);
    document.body.removeChild(a);
  },

  // Custom Reports
  listReportTemplates: (signal?: AbortSignal) =>
    get<{ templates: ReportTemplateResponse[] }>("/api/v1/reports/templates", {}, signal),
  createReportTemplate: async (payload: CreateReportTemplate): Promise<ReportTemplateResponse> => {
    const res = await fetch(`${BASE}/reports/templates`, {
      method: "POST",
      headers: {
        Accept: "application/json",
        "Content-Type": "application/json",
      },
      body: JSON.stringify(payload),
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
    const data = await res.json() as { template: ReportTemplateResponse };
    return data.template;
  },
  deleteReportTemplate: async (id: string): Promise<void> => {
    const res = await fetch(`${BASE}/reports/templates/${id}`, {
      method: "DELETE",
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
  },
  listCustomReports: (signal?: AbortSignal) =>
    get<{ reports: CustomReportResponse[] }>("/api/v1/reports/custom", {}, signal),
  createCustomReport: async (payload: CreateCustomReport): Promise<CustomReportResponse> => {
    const res = await fetch(`${BASE}/reports/custom`, {
      method: "POST",
      headers: {
        Accept: "application/json",
        "Content-Type": "application/json",
      },
      body: JSON.stringify(payload),
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
    const data = await res.json() as { report: CustomReportResponse };
    return data.report;
  },
  getCustomReport: async (id: string): Promise<CustomReportResponse> => {
    const res = await fetch(`${BASE}/reports/custom/${id}`, {
      method: "GET",
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
    const data = await res.json() as { report: CustomReportResponse };
    return data.report;
  },
  updateCustomReport: async (id: string, payload: CreateCustomReport): Promise<CustomReportResponse> => {
    const res = await fetch(`${BASE}/reports/custom/${id}`, {
      method: "PUT",
      headers: {
        Accept: "application/json",
        "Content-Type": "application/json",
      },
      body: JSON.stringify(payload),
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
    const data = await res.json() as { report: CustomReportResponse };
    return data.report;
  },
  deleteCustomReport: async (id: string): Promise<void> => {
    const res = await fetch(`${BASE}/reports/custom/${id}`, {
      method: "DELETE",
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
  },
  runCustomReport: async (id: string): Promise<{ result_id: string; status: string; file_path: string | null; generated_at: string }> => {
    const res = await fetch(`${BASE}/reports/custom/${id}/run`, {
      method: "POST",
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
    return (await res.json()) as { result_id: string; status: string; file_path: string | null; generated_at: string };
  },
};
