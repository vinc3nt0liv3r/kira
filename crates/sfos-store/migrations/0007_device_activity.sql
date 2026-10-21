CREATE TABLE IF NOT EXISTS device_activity (
    tenant_id uuid NOT NULL REFERENCES tenants (id) ON DELETE CASCADE,
    bucket    timestamptz NOT NULL,
    device_key text NOT NULL,
    name      text NOT NULL,
    device_id text,
    model     text,
    firmware  text,
    last_seen timestamptz NOT NULL,
    events    bigint NOT NULL,
    PRIMARY KEY (tenant_id, bucket, device_key)
);
