-- Tenants are first class even though the first release is single tenant: every
-- event row carries a tenant_id, so adding a second tenant is configuration,
-- not a migration.
CREATE TABLE IF NOT EXISTS tenants (
    id             uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    slug           text NOT NULL UNIQUE CHECK (slug ~ '^[a-z0-9][a-z0-9-]{0,62}$'),
    name           text NOT NULL,
    is_default     boolean NOT NULL DEFAULT false,
    retention_days integer NOT NULL DEFAULT 90 CHECK (retention_days BETWEEN 1 AND 3650),
    created_at     timestamptz NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX IF NOT EXISTS tenants_single_default
    ON tenants (is_default) WHERE is_default;

CREATE SEQUENCE IF NOT EXISTS events_id_seq;

-- One row per normalized syslog message. Partitioned by day so retention is a
-- metadata operation (DROP PARTITION) instead of a mass DELETE.
CREATE TABLE IF NOT EXISTS events (
    id                   bigint NOT NULL DEFAULT nextval('events_id_seq'),
    tenant_id            uuid NOT NULL REFERENCES tenants (id) ON DELETE CASCADE,
    ts                   timestamptz NOT NULL,
    received_at          timestamptz NOT NULL DEFAULT now(),
    device_id            text,
    device_name          text,
    log_id               text,
    log_type             text NOT NULL DEFAULT '',
    log_component        text,
    log_subtype          text,
    status               text,
    priority             text,
    category_group       text NOT NULL,
    action               text,
    src_ip               inet,
    dst_ip               inet,
    src_port             integer,
    dst_port             integer,
    protocol             text,
    src_country          text,
    dst_country          text,
    src_zone             text,
    dst_zone             text,
    user_name            text,
    user_group           text,
    domain               text,
    url                  text,
    http_method          text,
    http_status          integer,
    mime_type            text,
    referer              text,
    user_agent           text,
    category             text,
    application          text,
    application_category text,
    application_risk     integer,
    fw_rule_id           bigint,
    fw_rule_name         text,
    policy_type          text,
    sent_bytes           bigint,
    recv_bytes           bigint,
    threat_name          text,
    filename             text,
    reason               text,
    message              text,
    vpn_user             text,
    vpn_tunnel           text,
    attrs                jsonb NOT NULL DEFAULT '{}'::jsonb,
    PRIMARY KEY (ts, id)
) PARTITION BY RANGE (ts);

-- Catch events whose timestamp falls outside the pre-created daily partitions
-- (clock skew, delayed delivery, a source replaying yesterday's file).
CREATE TABLE IF NOT EXISTS events_default PARTITION OF events DEFAULT;

CREATE INDEX IF NOT EXISTS events_tenant_ts_idx ON events (tenant_id, ts DESC);
CREATE INDEX IF NOT EXISTS events_tenant_group_ts_idx ON events (tenant_id, category_group, ts DESC);
CREATE INDEX IF NOT EXISTS events_user_idx ON events (tenant_id, user_name, ts DESC)
    WHERE user_name IS NOT NULL;
