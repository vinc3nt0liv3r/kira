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
ASSET_FIELD = re.compile(r'(?:src|href)="(/assets/[^"]+)"')
RELEASE0_DEVICE = "X33010KMQVXTJ26"


def get_json(url: str) -> dict:
    with urllib.request.urlopen(url, timeout=10) as response:
        return json.load(response)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base-url", default="http://127.0.0.1:18082")
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=15514)
    parser.add_argument("--metrics-url", default="http://127.0.0.1:18083/metrics")
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
    captured_timestamps = [
        datetime.strptime(match.group(1), "%Y-%m-%dT%H:%M:%S%z").astimezone(timezone.utc)
        for line in lines
        if (match := TIMESTAMP_FIELD.search(line))
    ]
    if devices != {RELEASE0_DEVICE} or len(log_types) == 0 or len(captured_timestamps) != len(lines):
        print("FAIL  sample fixture has missing or inconsistent device/type/timestamp fields", file=sys.stderr)
        return 2
    device = next(iter(devices))
    server_time = get_json(args.base_url.rstrip("/") + "/api/v1/meta")["server_time"]
    now = datetime.fromisoformat(server_time.replace("Z", "+00:00")).astimezone(timezone.utc)
    now = now.replace(microsecond=0)
    timestamps = [now - timedelta(seconds=len(lines) - i) for i in range(len(lines))]
    lines = [
        TIMESTAMP_FIELD.sub(
            f'timestamp="{timestamp.strftime("%Y-%m-%dT%H:%M:%S%z")}"',
            line,
            count=1,
        )
        for line, timestamp in zip(lines, timestamps, strict=True)
    ]
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
        metrics_before = get_json(args.metrics_url)
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
    actual_groups = collections.Counter(row.get("group") for row in rows)
    actual_categories = {row.get("category") for row in rows if row.get("category")}
    expected_category_events = {
        "Information Technology": ("allowed", "Acceptable"),
        "Software Updates": ("blocked", "Acceptable"),
    }
    params = urllib.parse.urlencode(
        {"from": start.isoformat(), "to": end.isoformat()}
    )
    base = args.base_url.rstrip("/")
    overview = get_json(base + "/api/v1/overview?" + params)["totals"]
    destinations = get_json(
        base + "/api/v1/top?dimension=destinations&" + params
    )["rows"]
    threats = get_json(
        base + "/api/v1/top?dimension=threats&" + params
    )["rows"]
    auth_rows = get_json(
        base + "/api/v1/events?"
        + urllib.parse.urlencode(
            {"from": start.isoformat(), "to": end.isoformat(), "group": "auth"}
        )
    )["rows"]
    category_search_rows = get_json(
        base + "/api/v1/events?"
        + urllib.parse.urlencode(
            {"from": start.isoformat(), "to": end.isoformat(), "q": "Software Updates"}
        )
    )["rows"]
    meta = get_json(base + "/api/v1/meta")
    category_summary = get_json(
        base + "/api/v1/top?dimension=categories&" + params
    )["rows"]
    category_counts = {row["key"]: row["events"] for row in category_summary}
    category_events = {
        row.get("category"): (row.get("action"), row.get("category_type"))
        for row in rows
        if row.get("category") in expected_category_events
    }
    metrics_after = get_json(args.metrics_url)
    expected_groups = collections.Counter(
        {"firewall": 4, "web": 3, "threat": 1, "auth": 1}
    )
    checks = [
        ("all captured lines persisted", len(rows) == len(lines), f"{len(rows)}/{len(lines)}"),
        ("log types match fixture", actual_types == log_types, f"{actual_types} != {log_types}"),
        ("event groups match fixture", actual_groups == expected_groups,
         f"{actual_groups} != {expected_groups}"),
        ("HTTP categories match fixture", categories <= actual_categories,
         f"{actual_categories} does not include {categories}"),
        ("allowed and denied categories have expected action and type",
         category_events == expected_category_events,
         f"{category_events} != {expected_category_events}"),
        ("category text can be searched in the event API",
         any(row.get("category") == "Software Updates" for row in category_search_rows),
         "Software Updates search returned no matching event"),
    ]
    checks.extend([
        ("rollups are ready on a clean install",
         meta.get("rollups_ready") is True,
         f"rollups_ready={meta.get('rollups_ready')}"),
        ("recent devices come from ingested activity",
         any(item.get("device_id") == device for item in meta.get("devices", [])),
         f"devices={meta.get('devices', [])}"),
        ("overview event and threat totals match",
         overview["events"] == len(lines) and overview["threats"] == 1,
         f"{overview['events']} events, {overview['threats']} threats"),
        ("category summary reports the fixture categories",
         all(category_counts.get(category) == 1 for category in expected_category_events),
         f"{category_counts}"),
        ("application destination is reported",
         any(row["key"] == "Application: Windows Update" for row in destinations),
         "Windows Update not found in top destinations"),
        ("threat is reported",
         any(row["key"] == "EICAR-Test-File" for row in threats),
         "EICAR-Test-File not found in top threats"),
        ("authentication event is searchable",
         any(row["group"] == "auth" and row["user"] == "release0-admin" for row in auth_rows),
         "expected authentication event not found"),
        ("queue pressure metrics are exposed",
         all(key in metrics_after for key in (
             "queue_depth", "queue_capacity", "queue_utilization_percent", "queue_high_water"
         )),
         "one or more queue metrics are missing"),
        ("sample events were written without loss",
         metrics_after["written"] - metrics_before["written"] >= len(lines)
         and metrics_after["queued"] - metrics_before["queued"] >= len(lines)
         and metrics_after["dropped"] == metrics_before["dropped"]
         and metrics_after["db_errors"] == metrics_before["db_errors"],
         f"queued +{metrics_after['queued'] - metrics_before['queued']}, "
         f"written +{metrics_after['written'] - metrics_before['written']}, "
         f"dropped +{metrics_after['dropped'] - metrics_before['dropped']}, "
         f"db errors +{metrics_after['db_errors'] - metrics_before['db_errors']}"),
    ])
    check_metrics = (
        metrics_after["queue_capacity"] > 0
        and metrics_after["queue_depth"] <= metrics_after["queue_capacity"]
        and metrics_after["queue_high_water"] <= metrics_after["queue_capacity"]
        and metrics_after["queue_utilization_percent"] <= 100
    )
    checks.append((
        "queue pressure is within capacity",
        check_metrics,
        f"{metrics_after['queue_depth']}/{metrics_after['queue_capacity']} "
        f"({metrics_after['queue_utilization_percent']}%, high-water "
        f"{metrics_after['queue_high_water']})",
    ))
    try:
        base = args.base_url.rstrip("/")
        with urllib.request.urlopen(base + "/", timeout=15) as response:
            html = response.read().decode("utf-8", "replace")
        assets = ASSET_FIELD.findall(html)
        checks.append((
            "Kira dashboard and built assets are served",
            "Kira - The Sophos Reporter" in html and bool(assets),
            "expected product title or bundled assets missing",
        ))
        asset_statuses = []
        for asset in assets:
            with urllib.request.urlopen(base + asset, timeout=15) as response:
                asset_statuses.append(response.status)
        checks.append((
            "dashboard assets load",
            bool(asset_statuses) and all(status == 200 for status in asset_statuses),
            str(asset_statuses),
        ))
    except (OSError, TimeoutError, ValueError) as err:
        checks.append(("Kira dashboard assets load", False, str(err)))
    for name, passed, detail in checks:
        print(f"{'PASS' if passed else 'FAIL'}  {name}" + ("" if passed else f" [{detail}]"))
    return 0 if all(passed for _, passed, _ in checks) else 1


if __name__ == "__main__":
    raise SystemExit(main())
