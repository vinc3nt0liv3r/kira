export interface Tenant {
  slug: string;
  name: string;
  is_default: boolean;
  retention_days: number;
}

export interface Device {
  name: string;
  device_id: string | null;
  model: string | null;
  firmware: string | null;
  last_seen: string;
  events: number;
}

export interface Meta {
  tenants: Tenant[];
  tenant: Tenant;
  server_time: string;
  last_event_at: string | null;
  devices: Device[];
  raw_retention_hours: number;
  rollups_ready: boolean;
}

export interface Totals {
  events: number;
  web_requests: number;
  allowed: number;
  blocked: number;
  bytes_up: number;
  bytes_down: number;
  threats: number;
  users: number;
  domains: number;
}

export interface SeriesPoint {
  t: string;
  events: number;
  allowed: number;
  blocked: number;
  bytes_up: number;
  bytes_down: number;
}

export interface Overview {
  range: { from: string; to: string; bucket_secs: number };
  totals: Totals;
  series: SeriesPoint[];
}

export type Dimension =
  | "users"
  | "domains"
  | "destinations"
  | "categories"
  | "policies"
  | "threats"
  | "applications"
  | "countries"
  | "devices";

export interface TopRow {
  key: string;
  events: number;
  blocked: number;
  bytes: number;
  users: number | null;
}

export interface TopResponse {
  dimension: Dimension;
  rows: TopRow[];
}

export interface EventRow {
  id: number;
  ts: string;
  group: string;
  action: "allowed" | "blocked" | "warned" | null;
  log_type: string;
  log_component: string | null;
  device_name: string | null;
  src_ip: string | null;
  dst_ip: string | null;
  src_port: number | null;
  dst_port: number | null;
  user: string | null;
  domain: string | null;
  url: string | null;
  method: string | null;
  status: number | null;
  category: string | null;
  application: string | null;
  rule: string | null;
  bytes_up: number | null;
  bytes_down: number | null;
  threat: string | null;
  message: string | null;
}

export interface EventsResponse {
  range: { from: string; to: string };
  limit: number;
  offset: number;
  rows: EventRow[];
}
