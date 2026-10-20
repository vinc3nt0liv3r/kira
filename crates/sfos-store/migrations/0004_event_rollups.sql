CREATE TABLE IF NOT EXISTS event_rollups (
    tenant_id    uuid NOT NULL REFERENCES tenants (id) ON DELETE CASCADE,
    bucket       timestamptz NOT NULL,
    dimension    text NOT NULL,
    key          text NOT NULL,
    events       bigint NOT NULL DEFAULT 0,
    allowed      bigint NOT NULL DEFAULT 0,
    blocked      bigint NOT NULL DEFAULT 0,
    web_requests bigint NOT NULL DEFAULT 0,
    threats      bigint NOT NULL DEFAULT 0,
    bytes_up     bigint NOT NULL DEFAULT 0,
    bytes_down   bigint NOT NULL DEFAULT 0,
    PRIMARY KEY (tenant_id, bucket, dimension, key)
);

CREATE INDEX IF NOT EXISTS event_rollups_lookup_idx
    ON event_rollups (tenant_id, dimension, bucket, events DESC);

CREATE TABLE IF NOT EXISTS event_rollup_state (
    singleton       boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    history_ready   boolean NOT NULL DEFAULT false,
    updated_at      timestamptz NOT NULL DEFAULT now()
);

INSERT INTO event_rollup_state (singleton, history_ready)
VALUES (true, false)
ON CONFLICT (singleton) DO NOTHING;

CREATE OR REPLACE FUNCTION sfos_refresh_rollups(p_from timestamptz, p_to timestamptz)
RETURNS bigint
LANGUAGE plpgsql
AS $$
DECLARE
    changed bigint;
BEGIN
    IF p_from >= p_to OR date_trunc('hour', p_from) <> p_from
       OR date_trunc('hour', p_to) <> p_to THEN
        RAISE EXCEPTION 'rollup refresh range must be a non-empty whole-hour interval';
    END IF;

    DELETE FROM event_rollups
    WHERE bucket >= p_from AND bucket < p_to;

    WITH overview AS (
        SELECT tenant_id,
               date_trunc('hour', ts) AS bucket,
               count(*)::bigint AS events,
               count(*) FILTER (WHERE action = 'allowed'
                   AND COALESCE(con_event, '') <> 'Start')::bigint AS allowed,
               count(*) FILTER (WHERE action = 'blocked'
                   AND COALESCE(con_event, '') <> 'Start')::bigint AS blocked,
               count(*) FILTER (WHERE category_group = 'web'
                   AND COALESCE(con_event, '') <> 'Start')::bigint AS web_requests,
               count(*) FILTER (WHERE category_group = 'threat')::bigint AS threats,
               COALESCE(sum(sent_bytes), 0)::bigint AS bytes_up,
               COALESCE(sum(recv_bytes), 0)::bigint AS bytes_down
        FROM events
        WHERE ts >= p_from AND ts < p_to
        GROUP BY tenant_id, 2
    ), dimensions AS (
        SELECT e.tenant_id,
               date_trunc('hour', e.ts) AS bucket,
               d.dimension,
               d.key,
               count(*) FILTER (WHERE COALESCE(e.con_event, '') <> 'Start')::bigint AS events,
               count(*) FILTER (WHERE e.action = 'allowed'
                   AND COALESCE(e.con_event, '') <> 'Start')::bigint AS allowed,
               count(*) FILTER (WHERE e.action = 'blocked'
                   AND COALESCE(e.con_event, '') <> 'Start')::bigint AS blocked,
               count(*) FILTER (WHERE e.category_group = 'web'
                   AND COALESCE(e.con_event, '') <> 'Start')::bigint AS web_requests,
               count(*) FILTER (WHERE e.category_group = 'threat')::bigint AS threats,
               COALESCE(sum(e.sent_bytes) FILTER (WHERE COALESCE(e.con_event, '') <> 'Start'), 0)::bigint AS bytes_up,
               COALESCE(sum(e.recv_bytes) FILTER (WHERE COALESCE(e.con_event, '') <> 'Start'), 0)::bigint AS bytes_down
        FROM events e
        CROSS JOIN LATERAL (
            VALUES
                ('users', COALESCE(NULLIF(e.user_name, ''), host(e.src_ip))),
                ('domains', NULLIF(e.domain, '')),
                ('categories', NULLIF(e.category, '')),
                ('policies', NULLIF(e.fw_rule_name, '')),
                ('threats', CASE WHEN e.category_group = 'threat'
                    THEN COALESCE(NULLIF(e.threat_name, ''), NULLIF(e.reason, '')) END),
                ('applications', NULLIF(e.application, '')),
                ('countries', NULLIF(e.dst_country, '')),
                ('devices', COALESCE(NULLIF(e.device_name, ''), NULLIF(e.device_id, ''))),
                ('destinations', 'Website: ' || NULLIF(e.domain, '')),
                ('destinations', 'Application: ' || NULLIF(e.application, ''))
        ) AS d(dimension, key)
        WHERE e.ts >= p_from AND e.ts < p_to
          AND d.key IS NOT NULL AND d.key <> ''
        GROUP BY e.tenant_id, 2, d.dimension, d.key
    ), all_rollups AS (
        SELECT tenant_id, bucket, 'overview'::text AS dimension, ''::text AS key,
               events, allowed, blocked, web_requests, threats, bytes_up, bytes_down
        FROM overview
        UNION ALL
        SELECT tenant_id, bucket, dimension, key, events, allowed, blocked,
               web_requests, threats, bytes_up, bytes_down
        FROM dimensions
    )
    INSERT INTO event_rollups (
        tenant_id, bucket, dimension, key, events, allowed, blocked,
        web_requests, threats, bytes_up, bytes_down
    )
    SELECT tenant_id, bucket, dimension, key, events, allowed, blocked,
           web_requests, threats, bytes_up, bytes_down
    FROM all_rollups;

    GET DIAGNOSTICS changed = ROW_COUNT;
    RETURN changed;
END;
$$;
