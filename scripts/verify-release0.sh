#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

keep_stack=false
if [[ $# -gt 1 || ( $# -eq 1 && "$1" != "--keep" ) ]]; then
  echo "usage: $0 [--keep]" >&2
  exit 2
fi
if [[ $# -eq 1 ]]; then
  keep_stack=true
fi

project="kira-release0-$$"
export COMPOSE_PROJECT_NAME=$project
export HOST_BIND_IP=127.0.0.1
export API_PORT=18082
export SYSLOG_PORT=15514
export INGEST_HEALTH_PORT=18083
export RETENTION_HOURS=6
export RETENTION_ADMIN_TOKEN="kira-isolated-retention-test-token-not-for-production"

if [[ -n "$(docker compose -p "$project" ps -q 2>/dev/null)" ]]; then
  echo "Refusing to reuse an existing Compose project: $project" >&2
  exit 2
fi

cleanup() {
  status=$?
  trap - EXIT
  if [[ "$keep_stack" == true && $status -eq 0 ]]; then
    printf 'Keeping isolated test stack %s for dashboard inspection.\n' "$project"
    printf 'Stop and remove it with: COMPOSE_PROJECT_NAME=%s docker compose down --volumes --remove-orphans\n' "$project"
  elif ! docker compose -p "$project" down --volumes --remove-orphans; then
    echo "ERROR: failed to remove isolated Compose project $project" >&2
    status=1
  fi
  exit "$status"
}
trap cleanup EXIT

docker compose -p "$project" up -d --build --wait

for _ in $(seq 1 60); do
  if curl -fsS http://127.0.0.1:18082/healthz >/dev/null \
    && curl -fsS http://127.0.0.1:18083/healthz >/dev/null; then
    break
  fi
  sleep 2
done

curl -fsS http://127.0.0.1:18082/healthz >/dev/null
curl -fsS http://127.0.0.1:18083/healthz >/dev/null
python3 scripts/verify-samples.py \
  --yes \
  --base-url http://127.0.0.1:18082 \
  --host 127.0.0.1 \
  --port 15514 \
  --metrics-url http://127.0.0.1:18083/metrics
docker compose -p "$project" exec -T db psql -U sfos -d sfos -v ON_ERROR_STOP=1 \
  -c "ANALYZE events" >/dev/null
python3 scripts/verify-retention.py \
  --base-url http://127.0.0.1:18082 \
  --token "$RETENTION_ADMIN_TOKEN"
docker compose -p "$project" restart api ingest >/dev/null
for _ in $(seq 1 60); do
  if curl -fsS http://127.0.0.1:18082/healthz >/dev/null \
    && curl -fsS http://127.0.0.1:18083/healthz >/dev/null; then
    break
  fi
  sleep 1
done
python3 -c "import json, urllib.request; meta=json.load(urllib.request.urlopen('http://127.0.0.1:18082/api/v1/meta', timeout=10)); assert meta['raw_retention_hours'] == 6; print('PASS  global retention survives API and collector restart')"
python3 scripts/benchmark-release0.py
bash scripts/verify-backup-restore.sh

echo "PASS  isolated clean-install, sample replay, API, rollup, metrics, and backup/restore checks"
