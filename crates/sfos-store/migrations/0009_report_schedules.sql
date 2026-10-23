CREATE TABLE IF NOT EXISTS report_schedules (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id uuid NOT NULL REFERENCES tenants (id) ON DELETE CASCADE,
    name text NOT NULL,
    description text,
    cron_schedule text NOT NULL CHECK (cron_schedule <> ''),
    format text NOT NULL CHECK (format IN ('csv', 'pdf')),
    recipients jsonb NOT NULL DEFAULT '[]'::jsonb,
    filters jsonb NOT NULL DEFAULT '{}'::jsonb,
    enabled boolean NOT NULL DEFAULT true,
    last_run timestamptz,
    next_run timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS report_schedules_tenant_id_idx ON report_schedules (tenant_id);
CREATE INDEX IF NOT EXISTS report_schedules_next_run_idx ON report_schedules (next_run) WHERE enabled;

CREATE TABLE IF NOT EXISTS report_results (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    schedule_id uuid NOT NULL REFERENCES report_schedules (id) ON DELETE CASCADE,
    status text NOT NULL CHECK (status IN ('pending', 'generated', 'sent', 'failed')),
    file_path text,
    row_count bigint DEFAULT 0,
    error_message text,
    generated_at timestamptz NOT NULL DEFAULT now(),
    sent_at timestamptz
);

CREATE INDEX IF NOT EXISTS report_results_schedule_id_idx ON report_results (schedule_id);
CREATE INDEX IF NOT EXISTS report_results_generated_at_idx ON report_results (generated_at DESC);

CREATE TABLE IF NOT EXISTS alert_rules (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id uuid NOT NULL REFERENCES tenants (id) ON DELETE CASCADE,
    name text NOT NULL,
    description text,
    condition_type text NOT NULL CHECK (condition_type IN ('threat', 'blocked_category', 'large_transfer')),
    threshold text NOT NULL,
    severity text NOT NULL CHECK (severity IN ('low', 'medium', 'high', 'critical')) DEFAULT 'medium',
    cooldown_minutes integer NOT NULL DEFAULT 60 CHECK (cooldown_minutes > 0),
    enabled boolean NOT NULL DEFAULT true,
    last_triggered timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS alert_rules_tenant_id_idx ON alert_rules (tenant_id);
CREATE INDEX IF NOT EXISTS alert_rules_enabled_idx ON alert_rules (enabled) WHERE enabled;

CREATE TABLE IF NOT EXISTS alert_instances (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    rule_id uuid NOT NULL REFERENCES alert_rules (id) ON DELETE CASCADE,
    tenant_id uuid NOT NULL REFERENCES tenants (id) ON DELETE CASCADE,
    message text NOT NULL,
    severity text NOT NULL CHECK (severity IN ('low', 'medium', 'high', 'critical')),
    triggered_at timestamptz NOT NULL DEFAULT now(),
    acknowledged boolean NOT NULL DEFAULT false,
    acknowledged_at timestamptz,
    acknowledged_by text
);

CREATE INDEX IF NOT EXISTS alert_instances_rule_id_idx ON alert_instances (rule_id);
CREATE INDEX IF NOT EXISTS alert_instances_tenant_id_idx ON alert_instances (tenant_id);
CREATE INDEX IF NOT EXISTS alert_instances_triggered_at_idx ON alert_instances (triggered_at DESC);
CREATE INDEX IF NOT EXISTS alert_instances_unacknowledged_idx ON alert_instances (tenant_id, acknowledged) WHERE NOT acknowledged;
