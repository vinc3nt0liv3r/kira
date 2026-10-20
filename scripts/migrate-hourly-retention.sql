\set ON_ERROR_STOP on

BEGIN;
SELECT pg_advisory_xact_lock(1397116755);
SET LOCAL TIME ZONE 'UTC';
LOCK TABLE events IN ACCESS EXCLUSIVE MODE;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_inherits i
        JOIN pg_class c ON c.oid = i.inhrelid
        JOIN pg_class p ON p.oid = i.inhparent
        WHERE p.oid = 'events'::regclass
          AND c.relname ~ '^events_[0-9]{8}$'
    ) THEN
        RAISE EXCEPTION 'expected daily events partitions; this migration is only for the daily-to-hourly upgrade';
    END IF;
END;
$$;

CREATE TABLE events_live (LIKE events INCLUDING DEFAULTS INCLUDING CONSTRAINTS)
    PARTITION BY RANGE (ts);
ALTER TABLE events_live ADD PRIMARY KEY (ts, id);
ALTER TABLE events_live ADD CONSTRAINT events_live_tenant_fk
    FOREIGN KEY (tenant_id) REFERENCES tenants (id) ON DELETE CASCADE;
CREATE INDEX events_live_tenant_ts_idx ON events_live (tenant_id, ts DESC);
CREATE INDEX events_live_tenant_group_ts_idx
    ON events_live (tenant_id, category_group, ts DESC);
CREATE INDEX events_live_user_idx ON events_live (tenant_id, user_name, ts DESC)
    WHERE user_name IS NOT NULL;
CREATE TABLE events_live_default PARTITION OF events_live DEFAULT;

DO $$
DECLARE
    hour_start timestamptz;
    hour_end timestamptz;
    partition_name text;
BEGIN
    hour_start := date_trunc('hour', now() - interval '6 hours');
    WHILE hour_start <= date_trunc('hour', now()) + interval '48 hours' LOOP
        hour_end := hour_start + interval '1 hour';
        partition_name := 'events_' || to_char(hour_start AT TIME ZONE 'UTC', 'YYYYMMDDHH24');
        EXECUTE format(
            'CREATE TABLE %I PARTITION OF events_live FOR VALUES FROM (%L) TO (%L)',
            partition_name, hour_start, hour_end
        );
        hour_start := hour_end;
    END LOOP;
END;
$$;

INSERT INTO events_live
SELECT *
FROM events
WHERE ts >= now() - interval '6 hours';

TRUNCATE event_rollups;
DO $$
DECLARE
    first_day timestamptz;
    last_day timestamptz;
    day_start timestamptz;
BEGIN
    SELECT date_trunc('day', min(ts)),
           date_trunc('day', max(ts)) + interval '1 day'
    INTO first_day, last_day
    FROM events;

    IF first_day IS NOT NULL THEN
        day_start := first_day;
        WHILE day_start < last_day LOOP
            PERFORM sfos_refresh_rollups(day_start, day_start + interval '1 day');
            day_start := day_start + interval '1 day';
        END LOOP;
    END IF;
END;
$$;

DO $$
DECLARE
    source_total bigint;
    source_recent bigint;
    raw_recent bigint;
    summarized_total bigint;
BEGIN
    SELECT count(*),
           count(*) FILTER (WHERE ts >= now() - interval '6 hours')
    INTO source_total, source_recent
    FROM events;
    SELECT count(*) INTO raw_recent FROM events_live;
    SELECT COALESCE(sum(events), 0)
    INTO summarized_total
    FROM event_rollups
    WHERE dimension = 'overview';

    IF source_recent <> raw_recent OR source_total <> summarized_total THEN
        RAISE EXCEPTION
            'hourly migration verification failed: source %, copied raw %, summarized %',
            source_recent, raw_recent, summarized_total;
    END IF;
END;
$$;

ALTER TABLE events RENAME TO events_legacy;
ALTER TABLE events_live RENAME TO events;
DROP TABLE events_legacy;
ALTER TABLE events_live_default RENAME TO events_default;
ALTER INDEX events_live_tenant_ts_idx RENAME TO events_tenant_ts_idx;
ALTER INDEX events_live_tenant_group_ts_idx RENAME TO events_tenant_group_ts_idx;
ALTER INDEX events_live_user_idx RENAME TO events_user_idx;
UPDATE event_rollup_state
SET history_ready = true, updated_at = now()
WHERE singleton;

COMMIT;
