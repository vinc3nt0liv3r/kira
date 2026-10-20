#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 || "$1" != /* ]]; then
  printf 'Usage: %s /absolute/path/to/pre-migration.dump\n' "$0" >&2
  exit 2
fi

backup_path=$1
if [[ -e "$backup_path" ]]; then
  printf 'Refusing to overwrite existing backup: %s\n' "$backup_path" >&2
  exit 2
fi

running_services=$(docker compose ps -q api ingest)
if [[ -n "$running_services" ]]; then
  printf 'Stop the API and collector before running this migration.\n' >&2
  exit 2
fi

mkdir -p "$(dirname "$backup_path")"
docker compose up -d --wait db

printf 'Creating database backup at %s\n' "$backup_path"
docker compose exec -T db pg_dump -U sfos -d sfos -Fc --no-owner --no-acl > "$backup_path"
if [[ ! -s "$backup_path" ]]; then
  printf 'Backup is empty; refusing to continue.\n' >&2
  exit 1
fi

rollup_migration=$(docker compose exec -T db psql -U sfos -d sfos -At \
  -c "SELECT EXISTS (SELECT 1 FROM schema_migrations WHERE version = '0004_event_rollups')")
if [[ "$rollup_migration" != "t" ]]; then
  {
    printf 'BEGIN;\n'
    cat crates/sfos-store/migrations/0004_event_rollups.sql
    printf "\nINSERT INTO schema_migrations (version) VALUES ('0004_event_rollups');\nCOMMIT;\n"
  } | docker compose exec -T db psql -U sfos -d sfos -v ON_ERROR_STOP=1
fi

printf 'Converting daily raw partitions to hourly partitions and building rollups.\n'
docker compose exec -T db psql -U sfos -d sfos -v ON_ERROR_STOP=1 \
  < scripts/migrate-hourly-retention.sql

printf 'Migration complete. Backup retained at %s\n' "$backup_path"
