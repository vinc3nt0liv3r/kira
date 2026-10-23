ALTER TABLE alert_rules ADD COLUMN webhook_url text;

CREATE TABLE IF NOT EXISTS webhook_deliveries (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    alert_instance_id uuid NOT NULL REFERENCES alert_instances (id) ON DELETE CASCADE,
    webhook_url text NOT NULL,
    status text NOT NULL CHECK (status IN ('sent', 'failed')),
    response_status_code integer,
    response_body text,
    error_message text,
    delivered_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS webhook_deliveries_alert_instance_id_idx ON webhook_deliveries (alert_instance_id);
CREATE INDEX IF NOT EXISTS webhook_deliveries_delivered_at_idx ON webhook_deliveries (delivered_at DESC);
