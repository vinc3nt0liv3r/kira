#!/usr/bin/env bash
# Run the reporter without Docker: bundled Postgres + collector + API, all on
# localhost, with state kept in .runtime/ so logs survive restarts.
#
#   scripts/dev.sh start     build (if needed) and start everything
#   scripts/dev.sh stop      stop collector and API (database keeps running)
#   scripts/dev.sh status    show what is running and where logs go
#   scripts/dev.sh logs      tail collector + API logs
#
# The database listens on 127.0.0.1:55432 only; syslog and the dashboard are
# exposed on all interfaces so a firewall on the LAN can reach them.
set -euo pipefail

cd "$(dirname "$0")/.."

RUNTIME="$PWD/.runtime"
PGBIN=$(echo "$RUNTIME"/postgres/postgresql-*/bin)
export LD_LIBRARY_PATH="$(dirname "$PGBIN")/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"

DATABASE_URL=${DATABASE_URL:-postgres://postgres@127.0.0.1:55432/postgres}
SYSLOG_PORT=${SYSLOG_PORT:-5514}
API_PORT=${API_PORT:-8080}
HEALTH_PORT=${HEALTH_PORT:-8081}
TENANT_SLUG=${TENANT_SLUG:-default}
TENANT_NAME=${TENANT_NAME:-Head office}
RUST_LOG=${RUST_LOG:-info}

mkdir -p "$RUNTIME/logs" "$RUNTIME/run"

pg_running() {
  "$PGBIN/pg_ctl" -D "$RUNTIME/pgdata" status >/dev/null 2>&1
}

start_pg() {
  if pg_running; then
    return
  fi
  echo "starting postgres…"
  "$PGBIN/pg_ctl" -D "$RUNTIME/pgdata" -l "$RUNTIME/logs/postgres.log" \
    -o "-p 55432 -k $RUNTIME/run -c listen_addresses=127.0.0.1" -w start
}

stop_pg() {
  if pg_running; then
    "$PGBIN/pg_ctl" -D "$RUNTIME/pgdata" -m fast -w stop
  fi
}

start_app() {
  if pgrep -x sfos-ingest >/dev/null || pgrep -x sfos-api >/dev/null; then
    echo "collector or API already running; run '$0 stop' first" >&2
    exit 1
  fi

  cargo build --release --locked -q

  DATABASE_URL="$DATABASE_URL" TENANT_SLUG="$TENANT_SLUG" TENANT_NAME="$TENANT_NAME" \
    SYSLOG_UDP_ADDR="0.0.0.0:$SYSLOG_PORT" SYSLOG_TCP_ADDR="0.0.0.0:$SYSLOG_PORT" \
    HEALTH_ADDR="0.0.0.0:$HEALTH_PORT" RUST_LOG="$RUST_LOG" \
    setsid nohup ./target/release/sfos-ingest >>"$RUNTIME/logs/ingest.log" 2>&1 &
  echo $! >"$RUNTIME/run/ingest.pid"

  DATABASE_URL="$DATABASE_URL" API_ADDR="0.0.0.0:$API_PORT" WEB_DIR=web/dist \
    TENANT_SLUG="$TENANT_SLUG" RUST_LOG="$RUST_LOG" \
    setsid nohup ./target/release/sfos-api >>"$RUNTIME/logs/api.log" 2>&1 &
  echo $! >"$RUNTIME/run/api.pid"

  for _ in $(seq 1 40); do
    if curl -fsS "http://127.0.0.1:$API_PORT/healthz" >/dev/null 2>&1; then
      break
    fi
    sleep 0.25
  done
}

stop_app() {
  for name in sfos-api sfos-ingest; do
    pkill -TERM -x "$name" 2>/dev/null || true
  done
  for _ in $(seq 1 40); do
    pgrep -x sfos-api >/dev/null || pgrep -x sfos-ingest >/dev/null || break
    sleep 0.25
  done
  rm -f "$RUNTIME/run"/*.pid
}

case "${1:-start}" in
  start)
    start_pg
    if [ ! -f web/dist/index.html ]; then
      echo "building dashboard…"
      (cd web && npm install --no-audit --no-fund --silent && npm run build --silent)
    fi
    start_app
    echo
    echo "dashboard   http://localhost:$API_PORT  (LAN: http://$(hostname -I | awk '{print $1}'):$API_PORT)"
    echo "syslog      udp+tcp $SYSLOG_PORT"
    echo "collector   http://localhost:$HEALTH_PORT/metrics"
    ;;
  stop)
    stop_app
    echo "collector and API stopped (database still running; '$0 stop --all' to stop it)"
    if [ "${2:-}" = "--all" ]; then
      stop_pg
    fi
    ;;
  status)
    pg_running && echo "postgres    running (pid $(head -1 "$RUNTIME/pgdata/postmaster.pid"))" || echo "postgres    stopped"
    pgrep -x sfos-ingest >/dev/null && echo "collector   running" || echo "collector   stopped"
    pgrep -x sfos-api >/dev/null && echo "api         running" || echo "api         stopped"
    if curl -fsS "http://127.0.0.1:$API_PORT/healthz" >/dev/null 2>&1; then
      echo "health      ok"
    fi
    echo
    echo "listening:"
    ss -lntu 2>/dev/null | grep -E ":$SYSLOG_PORT|:$API_PORT|:$HEALTH_PORT" || true
    echo
    curl -fsS "http://127.0.0.1:$HEALTH_PORT/metrics" 2>/dev/null || true
    echo
    ;;
  logs)
    tail -n 40 -f "$RUNTIME/logs/ingest.log" "$RUNTIME/logs/api.log"
    ;;
  *)
    echo "usage: $0 {start|stop [--all]|status|logs}" >&2
    exit 2
    ;;
esac
