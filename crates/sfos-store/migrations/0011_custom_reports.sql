-- Report templates (pre-defined filter sets for custom reports)
CREATE TABLE IF NOT EXISTS report_templates (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id uuid NOT NULL REFERENCES tenants (id) ON DELETE CASCADE,
    name text NOT NULL,
    description text,
    category text NOT NULL DEFAULT 'custom',
    default_filters jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS report_templates_tenant_id_idx ON report_templates (tenant_id);

-- Add report_type to report_schedules to distinguish scheduled vs custom reports
ALTER TABLE report_schedules ADD COLUMN IF NOT EXISTS report_type text NOT NULL DEFAULT 'scheduled' CHECK (report_type IN ('scheduled', 'custom'));

CREATE INDEX IF NOT EXISTS report_schedules_report_type_idx ON report_schedules (report_type);

-- Seed default templates for the default tenant
INSERT INTO report_templates (tenant_id, name, description, category, default_filters)
SELECT t.id, 'Top Blocked Sites', 'Most frequently blocked web destinations', 'web_filter',
       '{"action": "blocked", "group": "destination", "from": "24h", "to": "now"}'::jsonb
FROM tenants t WHERE t.slug = 'default'
ON CONFLICT DO NOTHING;

INSERT INTO report_templates (tenant_id, name, description, category, default_filters)
SELECT t.id, 'Top Threat Events', 'Highest severity threat detections', 'security',
       '{"category_group": "threat", "from": "24h", "to": "now"}'::jsonb
FROM tenants t WHERE t.slug = 'default'
ON CONFLICT DO NOTHING;

INSERT INTO report_templates (tenant_id, name, description, category, default_filters)
SELECT t.id, 'User Activity Summary', 'Activity by user over the last 7 days', 'user_activity',
       '{"from": "7d", "to": "now", "group": "user"}'::jsonb
FROM tenants t WHERE t.slug = 'default'
ON CONFLICT DO NOTHING;

INSERT INTO report_templates (tenant_id, name, description, category, default_filters)
SELECT t.id, 'Bandwidth by Category', 'Data transfer volume by web category', 'bandwidth',
       '{"from": "24h", "to": "now", "group": "category"}'::jsonb
FROM tenants t WHERE t.slug = 'default'
ON CONFLICT DO NOTHING;
