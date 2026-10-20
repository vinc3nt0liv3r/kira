#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

if [ "$#" -ne 0 ]; then
  echo "usage: $0" >&2
  exit 2
fi

docker compose exec -T db psql -U sfos -d sfos -X -v ON_ERROR_STOP=1 -P pager=off <<'SQL'
SET default_transaction_read_only = on;
SET statement_timeout = '120s';

SELECT current_database() AS database,
       pg_size_pretty(pg_database_size(current_database())) AS database_size,
       pg_database_size(current_database()) AS database_bytes;

WITH partition_storage AS (
    SELECT inhrelid,
           pg_total_relation_size(inhrelid) AS bytes
    FROM pg_inherits
    WHERE inhparent = 'events'::regclass
),
event_totals AS (
    SELECT count(*)::bigint AS events,
           min(received_at) AS first_received,
           max(received_at) AS last_received
    FROM events
),
measurements AS (
    SELECT events,
           first_received,
           last_received,
           extract(epoch FROM (last_received - first_received)) AS span_seconds,
           COALESCE((SELECT sum(bytes) FROM partition_storage), 0) AS event_storage_bytes
    FROM event_totals
)
SELECT events,
       first_received,
       last_received,
       round(events / NULLIF(span_seconds, 0), 1) AS average_events_per_second,
       pg_size_pretty(event_storage_bytes) AS event_storage_including_indexes,
       round(event_storage_bytes::numeric / NULLIF(events, 0), 1) AS bytes_per_event_including_indexes,
       round(
           (event_storage_bytes::numeric / NULLIF(events, 0))
           * (events / NULLIF(span_seconds, 0))
           * 86400 / 1073741824,
           1
       ) AS projected_event_storage_gib_per_day
FROM measurements;

SELECT inhrelid::regclass AS partition,
       pg_size_pretty(pg_total_relation_size(inhrelid)) AS total_size,
       pg_size_pretty(pg_relation_size(inhrelid)) AS heap_size,
       pg_size_pretty(pg_indexes_size(inhrelid)) AS index_size
FROM pg_inherits
WHERE inhparent = 'events'::regclass
ORDER BY pg_total_relation_size(inhrelid) DESC
LIMIT 10;
SQL
