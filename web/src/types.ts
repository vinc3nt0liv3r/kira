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

export interface RetentionPreview {
  global: boolean;
  current_hours: number;
  choices_hours: number[];
  database_bytes: number;
  current_raw_partition_bytes: number;
  estimated_raw_bytes_per_hour: number | null;
  options: Array<{ hours: number; estimated_database_bytes: number | null }>;
  admin_configured: boolean;
  application_delay_minutes: number;
  estimate_note: string;
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
  category_type: string | null;
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

export interface EventsFilters {
  tenant: string;
  from: string;
  q?: string;
  action?: string;
  group?: string;
  user?: string;
  category?: string;
  device?: string;
}

export interface AlertRule {
  id: string;
  tenant_id: string;
  name: string;
  description: string | null;
  condition_type: string;
  threshold: string;
  severity: string;
  cooldown_minutes: number;
  enabled: boolean;
  webhook_url: string | null;
  last_triggered: string | null;
  created_at: string;
  updated_at: string;
}

export interface CreateAlertRule {
  name: string;
  description?: string | null;
  condition_type: string;
  threshold: string;
  severity?: string | null;
  cooldown_minutes?: number;
  enabled?: boolean;
  webhook_url?: string | null;
}

export interface UpdateAlertRule {
  name?: string;
  description?: string | null;
  condition_type?: string;
  threshold?: string;
  severity?: string | null;
  cooldown_minutes?: number;
  enabled?: boolean;
  webhook_url?: string | null;
}

export interface AlertRuleResponse {
  id: string;
  tenant_id: string;
  name: string;
  description: string | null;
  condition_type: string;
  threshold: string;
  severity: string;
  cooldown_minutes: number;
  enabled: boolean;
  webhook_url: string | null;
  last_triggered: string | null;
  created_at: string;
  updated_at: string;
}

export interface WebhookDelivery {
  id: string;
  alert_instance_id: string;
  webhook_url: string;
  status: string;
  response_status_code: number | null;
  response_body: string | null;
  error_message: string | null;
  delivered_at: string;
}

export interface AlertInstance {
  id: string;
  rule_id: string;
  tenant_id: string;
  message: string;
  severity: string;
  triggered_at: string;
  acknowledged: boolean;
  acknowledged_at: string | null;
  acknowledged_by: string | null;
}

export interface ReportSchedule {
  id: string;
  tenant_id: string;
  name: string;
  description: string | null;
  cron_schedule: string;
  format: string;
  filters: Record<string, unknown>;
  recipients: string[];
  enabled: boolean;
  last_run: string | null;
  next_run: string | null;
  created_at: string;
  updated_at: string;
}

export interface CreateReportSchedule {
  name: string;
  description?: string | null;
  cron_schedule: string;
  format: string;
  filters?: Record<string, unknown>;
  recipients?: string[];
  enabled?: boolean;
}

export interface UpdateReportSchedule {
  name?: string;
  description?: string | null;
  cron_schedule?: string;
  format?: string;
  filters?: Record<string, unknown>;
  recipients?: string[];
  enabled?: boolean;
}

export interface ReportScheduleResponse {
  id: string;
  tenant_id: string;
  name: string;
  description: string | null;
  cron_schedule: string;
  format: string;
  filters: Record<string, unknown>;
  recipients: string[];
  enabled: boolean;
  last_run: string | null;
  next_run: string | null;
  created_at: string;
  updated_at: string;
}

export interface ReportResult {
  id: string;
  schedule_id: string;
  tenant_id: string;
  status: string;
  file_path: string | null;
  row_count: number | null;
  error_message: string | null;
  generated_at: string;
  sent_at: string | null;
}

export interface TestWebhookResult {
  status: string;
  code: number;
  body: string;
}
