#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

if [ "$#" -ne 0 ]; then
  echo "usage: $0" >&2
  exit 2
fi

restore_db="sfos_restore_test_$$"
backup=$(mktemp "${TMPDIR:-/tmp}/sfos-backup-restore.XXXXXX")

cleanup() {
  status=$?
  trap - EXIT
  if ! docker compose exec -T db dropdb --if-exists -U sfos "$restore_db"; then
    echo "ERROR: could not drop scratch database $restore_db" >&2
    status=1
  fi
  if ! rm -f -- "$backup"; then
    echo "ERROR: could not remove temporary backup $backup" >&2
    status=1
  fi
  exit "$status"
}
trap cleanup EXIT

source_counts=$(docker compose exec -T db psql -U sfos -d sfos -X -At -F ',' \
  -v ON_ERROR_STOP=1 \
  -c "SELECT (SELECT count(*) FROM events), (SELECT count(*) FROM tenants), (SELECT count(*) FROM schema_migrations)")
docker compose exec -T db pg_dump -U sfos --format=custom --compress=9 sfos >"$backup"
docker compose exec -T db createdb -U sfos "$restore_db"
docker compose exec -T db pg_restore -U sfos --no-owner --no-acl --exit-on-error \
  -d "$restore_db" <"$backup"
restored_counts=$(docker compose exec -T db psql -U sfos -d "$restore_db" -X -At -F ',' \
  -v ON_ERROR_STOP=1 \
  -c "SELECT (SELECT count(*) FROM events), (SELECT count(*) FROM tenants), (SELECT count(*) FROM schema_migrations)")

if [ "$source_counts" != "$restored_counts" ]; then
  echo "FAIL  restored database counts differ (source=$source_counts restored=$restored_counts)" >&2
  exit 1
fi

echo "PASS  backup restored into $restore_db (events, tenants, and migrations match: $source_counts)"
echo "backup archive size: $(du -h "$backup" | cut -f1)"
