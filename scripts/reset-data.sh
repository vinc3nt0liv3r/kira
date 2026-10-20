#!/usr/bin/env bash
# Delete every collected event while keeping the schema, tenants and settings.
# Use this to clear demo data or start a fresh reporting window.
#
#   scripts/reset-data.sh              local dev database (scripts/dev.sh)
#   scripts/reset-data.sh --docker     docker compose database
#   DATABASE_URL=postgres://… scripts/reset-data.sh
#
# This cannot be undone. Pass --yes to skip the confirmation prompt.
set -euo pipefail

cd "$(dirname "$0")/.."

DOCKER=0
ASSUME_YES=0
DEMO_ONLY=0
for arg in "$@"; do
  case "$arg" in
    --docker) DOCKER=1 ;;
    --yes|-y) ASSUME_YES=1 ;;
    --demo-only) DEMO_ONLY=1 ;;
    *) echo "unknown option $arg" >&2; exit 2 ;;
  esac
done

if [ "$DOCKER" = 1 ]; then
  DB=(docker compose exec -T db psql -U sfos -d sfos)
  LABEL="docker compose database"
elif [ -n "${DATABASE_URL:-}" ]; then
  DB=(psql "$DATABASE_URL")
  LABEL="$DATABASE_URL"
else
  PGBIN=$(echo .runtime/postgres/postgresql-*/bin)
  export LD_LIBRARY_PATH="$(dirname "$PGBIN")/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
  DB=("$PGBIN/psql" -h 127.0.0.1 -p 55432 -U postgres -d postgres)
  LABEL="local dev database"
fi

TENANT_NAME=${TENANT_NAME:-"Head office"}

if [ "$ASSUME_YES" != 1 ]; then
  if [ "$DEMO_ONLY" = 1 ]; then
    printf 'Delete synthetic DEMO-* events from %s? [y/N] ' "$LABEL"
  else
    printf 'Delete ALL events from %s? [y/N] ' "$LABEL"
  fi
  read -r reply
  case "$reply" in
    y|Y|yes|YES) ;;
    *) echo "aborted"; exit 1 ;;
  esac
fi

if [ "$DEMO_ONLY" = 1 ]; then
  "${DB[@]}" -v ON_ERROR_STOP=1 <<'SQL'
BEGIN;
DELETE FROM events WHERE device_name LIKE 'DEMO-%' OR device_id LIKE 'DEMO%';
COMMIT;
SELECT 'demo events remaining: ' || count(*) FROM events
    WHERE device_name LIKE 'DEMO-%' OR device_id LIKE 'DEMO%';
SELECT 'real events kept: ' || count(*) FROM events;
SQL
else
  "${DB[@]}" -v ON_ERROR_STOP=1 -v tenant_name="$TENANT_NAME" <<'SQL'
BEGIN;
TRUNCATE events;
ALTER SEQUENCE events_id_seq RESTART WITH 1;
DELETE FROM tenants WHERE NOT is_default;
UPDATE tenants SET name = :'tenant_name' WHERE is_default;
COMMIT;
SELECT 'events remaining: ' || count(*) FROM events;
SELECT slug, name FROM tenants ORDER BY slug;
SQL
fi

echo
echo "Done. New logs will appear in the dashboard as they arrive."
