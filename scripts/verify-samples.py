#!/usr/bin/env python3
"""Replay captured Sophos samples and verify the persisted API results.

Run against an isolated/test database. This sends the captured sample events to
the configured collector; it does not clear existing events.
"""

from __future__ import annotations

import argparse
import collections
import json
import re
import socket
import sys
import time
import urllib.parse
import urllib.request
from datetime import datetime, timedelta, timezone
from pathlib import Path

SAMPLE_FILE = Path(__file__).resolve().parents[1] / "fw1_samples.log"
DEVICE_FIELD = re.compile(r'\bdevice_name="((?:\\.|[^"])*)"')
LOG_TYPE_FIELD = re.compile(r'\blog_type="((?:\\.|[^"])*)"')
CATEGORY_FIELD = re.compile(r'\bhttp_category="((?:\\.|[^"])*)"')
TIMESTAMP_FIELD = re.compile(r'\btimestamp="([^"]+)"')


def get_json(url: str) -> dict:
    with urllib.request.urlopen(url, timeout=10) as response:
        return json.load(response)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base-url", default="http://127.0.0.1:18082")
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=15514)
    parser.add_argument("--timeout", type=int, default=30)
    parser.add_argument("--yes", action="store_true", help="confirm writing captured events")
    args = parser.parse_args()

    if not args.yes:
        print("This writes captured sample events to the configured database; pass --yes.", file=sys.stderr)
        return 2

    lines = [
        line.strip()
        for line in SAMPLE_FILE.read_text(encoding="utf-8").splitlines()
        if line.strip() and not line.lstrip().startswith("#")
    ]
    if not lines:
        print(f"FAIL  no sample lines found in {SAMPLE_FILE}", file=sys.stderr)
        return 2

    devices = {match.group(1) for line in lines if (match := DEVICE_FIELD.search(line))}
    log_types = collections.Counter(
        match.group(1) for line in lines if (match := LOG_TYPE_FIELD.search(line))
    )
    categories = {
        match.group(1) for line in lines if (match := CATEGORY_FIELD.search(line))
    }
    timestamps = [
        datetime.strptime(match.group(1), "%Y-%m-%dT%H:%M:%S%z").astimezone(timezone.utc)
        for line in lines
        if (match := TIMESTAMP_FIELD.search(line))
    ]
    if len(devices) != 1 or len(log_types) == 0 or len(timestamps) != len(lines):
        print("FAIL  sample fixture has missing or inconsistent device/type/timestamp fields", file=sys.stderr)
        return 2
    device = next(iter(devices))
    start = min(timestamps) - timedelta(minutes=1)
    end = max(timestamps) + timedelta(minutes=1)
    query = urllib.parse.urlencode(
        {
            "from": start.isoformat().replace("+00:00", "Z"),
            "to": end.isoformat().replace("+00:00", "Z"),
            "limit": 500,
        }
    )
    endpoint = args.base_url.rstrip("/") + "/api/v1/events?" + query

    try:
        existing = [
            row for row in get_json(endpoint)["rows"] if row.get("device_name") == device
        ]
        if existing:
            print(
                f"FAIL  found {len(existing)} existing rows for {device} in the fixture time window; "
                "use an empty test database",
                file=sys.stderr,
            )
            return 2

        payload = ("\n".join(lines) + "\n").encode("utf-8")
        with socket.create_connection((args.host, args.port), timeout=10) as connection:
            connection.sendall(payload)
            connection.shutdown(socket.SHUT_WR)

        deadline = time.monotonic() + args.timeout
        rows = []
        while time.monotonic() < deadline:
            rows = [
                row for row in get_json(endpoint)["rows"] if row.get("device_name") == device
            ]
            if len(rows) >= len(lines):
                break
            time.sleep(0.5)
    except (OSError, TimeoutError, ValueError) as err:
        print(f"FAIL  sample replay or API request failed: {err}", file=sys.stderr)
        return 2

    actual_types = collections.Counter(row.get("log_type") for row in rows)
    actual_categories = {row.get("category") for row in rows if row.get("category")}
    checks = [
        ("all captured lines persisted", len(rows) == len(lines), f"{len(rows)}/{len(lines)}"),
        ("log types match fixture", actual_types == log_types, f"{actual_types} != {log_types}"),
        ("HTTP categories match fixture", categories <= actual_categories,
         f"{actual_categories} does not include {categories}"),
    ]
    for name, passed, detail in checks:
        print(f"{'PASS' if passed else 'FAIL'}  {name}" + ("" if passed else f" [{detail}]"))
    return 0 if all(passed for _, passed, _ in checks) else 1


if __name__ == "__main__":
    raise SystemExit(main())
