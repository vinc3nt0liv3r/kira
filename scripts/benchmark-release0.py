#!/usr/bin/env python3
"""Measure a deterministic 10,001-event load on an isolated Compose stack."""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import time
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
METRICS_URL = "http://127.0.0.1:18083/metrics"
EXPECTED_EVENTS = 10_001


def get_metrics() -> dict:
    with urllib.request.urlopen(METRICS_URL, timeout=10) as response:
        return json.load(response)


def event_storage_bytes() -> int:
    result = subprocess.run(
        [
            "docker", "compose", "exec", "-T", "db", "psql", "-U", "sfos", "-d", "sfos",
            "-X", "-At", "-v", "ON_ERROR_STOP=1", "-c",
            "SELECT COALESCE(sum(pg_total_relation_size(inhrelid)), 0) "
            "FROM pg_inherits WHERE inhparent = 'events'::regclass",
        ],
        cwd=ROOT,
        env=os.environ.copy(),
        check=True,
        capture_output=True,
        text=True,
    )
    return int(result.stdout.strip())


def main() -> int:
    before = get_metrics()
    storage_before = event_storage_bytes()
    with tempfile.NamedTemporaryFile(suffix=".json", delete=False) as handle:
        summary_path = Path(handle.name)
    started = time.monotonic()
    try:
        subprocess.run(
            [
                sys.executable,
                str(ROOT / "scripts/seed-demo.py"),
                "--yes",
                "--host", "127.0.0.1",
                "--port", "15514",
                "--web", "7000",
                "--firewall", "2000",
                "--threats", "500",
                "--auth", "300",
                "--vpn", "200",
                "--hours", "1",
                "--no-noise",
                "--summary-file", str(summary_path),
            ],
            cwd=ROOT,
            check=True,
            capture_output=True,
            text=True,
        )
        summary = json.loads(summary_path.read_text(encoding="utf-8"))
        if summary["accepted_lines"] != EXPECTED_EVENTS:
            raise RuntimeError(
                f"benchmark generated {summary['accepted_lines']} events, "
                f"expected {EXPECTED_EVENTS}"
            )

        deadline = started + 60
        after = get_metrics()
        while after["written"] - before["written"] < EXPECTED_EVENTS:
            if time.monotonic() >= deadline:
                raise TimeoutError(
                    f"collector wrote only {after['written'] - before['written']} "
                    f"of {EXPECTED_EVENTS} benchmark events"
                )
            time.sleep(0.5)
            after = get_metrics()
        elapsed = time.monotonic() - started
        storage_after = event_storage_bytes()
    finally:
        summary_path.unlink(missing_ok=True)

    dropped = after["dropped"] - before["dropped"]
    db_errors = after["db_errors"] - before["db_errors"]
    if dropped or db_errors:
        raise RuntimeError(
            f"benchmark lost events or had database errors: dropped={dropped}, "
            f"db_errors={db_errors}"
        )

    storage_delta = storage_after - storage_before
    if storage_delta <= 0:
        raise RuntimeError(f"event partition storage did not grow: delta={storage_delta}")

    report = {
        "events": EXPECTED_EVENTS,
        "seconds_to_persist": round(elapsed, 3),
        "events_per_second": round(EXPECTED_EVENTS / elapsed, 1),
        "event_storage_bytes_including_indexes": storage_delta,
        "bytes_per_event_including_indexes": round(storage_delta / EXPECTED_EVENTS, 1),
        "dropped": dropped,
        "database_write_errors": db_errors,
        "queue_capacity": after["queue_capacity"],
        "queue_high_water": after["queue_high_water"],
    }
    print(json.dumps(report, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
