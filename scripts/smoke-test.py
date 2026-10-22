#!/usr/bin/env python3
"""End-to-end smoke test: seed syslog, then verify what the dashboard reports.

Works against a running stack. Generates a small batch of events, waits for the
collector to write them, and checks the API's aggregates, filters and UI.

    python3 scripts/smoke-test.py --base-url http://127.0.0.1:8080 \
        --host 127.0.0.1 --port 5514
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request

ADDITIVE = ("events", "web_requests", "allowed", "blocked", "bytes_up", "bytes_down", "threats")


def get(base: str, path: str) -> dict:
    with urllib.request.urlopen(base + path, timeout=15) as response:
        return json.load(response)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base-url", default="http://127.0.0.1:8080")
    parser.add_argument("--host", default="127.0.0.1", help="syslog collector host")
    parser.add_argument("--port", type=int, default=5514, help="syslog collector port")
    parser.add_argument("--web", type=int, default=120, help="web events to generate")
    parser.add_argument("--timeout", type=int, default=30, help="seconds to wait for ingestion")
    parser.add_argument(
        "--allow-existing-data",
        action="store_true",
        help="run even if real (non-DEMO) firewalls already sent data to this collector",
    )
    args = parser.parse_args()

    base = args.base_url.rstrip("/")
    failures: list[str] = []
    # A fixed window: a rolling `from=24h` loses old events while the test runs,
    # which would make the ingestion deltas smaller than what was sent.
    window_from = int(time.time()) - 7 * 86_400
    window_to = int(time.time()) + 3_600
    window = f"from={window_from}&to={window_to}"

    def check(name: str, ok: bool, detail: str = "") -> None:
        print(("PASS  " if ok else "FAIL  ") + name + ("" if ok else f"   [{detail}]"))
        if not ok:
            failures.append(name)

    try:
        before = get(base, f"/api/v1/overview?{window}")["totals"]
        meta = get(base, "/api/v1/meta")
    except (urllib.error.URLError, TimeoutError) as err:
        print(f"FAIL  cannot reach {base}: {err}", file=sys.stderr)
        return 2

    real_devices = [
        device
        for device in meta.get("devices", [])
        if not str(device.get("name", "")).startswith("DEMO-")
    ]
    if real_devices and not args.allow_existing_data:
        names = ", ".join(str(device.get("name")) for device in real_devices)
        print(
            f"FAIL  {base} is collecting real firewall data ({names}); the smoke test would mix\n"
            "      synthetic events into it. Use a scratch collector, or pass\n"
            "      --allow-existing-data if you really mean to write here.",
            file=sys.stderr,
        )
        return 2

    seed = os.path.join(os.path.dirname(os.path.abspath(__file__)), "seed-demo.py")
    with tempfile.NamedTemporaryFile(suffix=".json", delete=False) as handle:
        summary_path = handle.name
    try:
        result = subprocess.run(
            [
                sys.executable,
                seed,
                "--yes",
                "--host",
                args.host,
                "--port",
                str(args.port),
                "--web",
                str(args.web),
                "--firewall",
                "40",
                "--threats",
                "8",
                "--auth",
                "6",
                "--vpn",
                "4",
                "--hours",
                "24",
                "--summary-file",
                summary_path,
            ],
            check=True,
            capture_output=True,
            text=True,
        )
        summary = json.load(open(summary_path, encoding="utf-8"))
    finally:
        os.unlink(summary_path)

    deadline = time.time() + args.timeout
    overview = get(base, f"/api/v1/overview?{window}")
    while overview["totals"]["events"] - before["events"] < summary["events"]:
        if time.time() > deadline:
            break
        time.sleep(1)
        overview = get(base, f"/api/v1/overview?{window}")

    after = overview["totals"]
    for key in ADDITIVE:
        delta = after[key] - before[key]
        check(f"ingested {key}: +{delta}", delta == summary[key], f"expected +{summary[key]}")
    for key in ("users", "domains"):
        check(f"{key} present", after[key] >= before[key] and after[key] > 0, str(after[key]))

    series = overview["series"]
    check("series buckets present", len(series) > 0, str(len(series)))
    for key in ("events", "allowed", "blocked", "bytes_up", "bytes_down"):
        total = sum(point[key] for point in series)
        check(f"series {key} sums to totals", total == after[key], f"{total} != {after[key]}")

    meta = get(base, "/api/v1/meta")
    check("collector received events", meta["last_event_at"] is not None)
    check("device discovered", len(meta["devices"]) > 0, str(meta["devices"]))

    top = get(base, f"/api/v1/top?{window}&dimension=users&limit=8")["rows"]
    check("top users ranked", all(top[i]["events"] >= top[i + 1]["events"] for i in range(len(top) - 1)))
    check("top users non-empty", len(top) > 0)

    blocked = get(base, f"/api/v1/events?{window}&action=blocked&limit=25")
    check("blocked filter only blocked", all(row["action"] == "blocked" for row in blocked["rows"]))
    check("blocked filter returns rows", len(blocked["rows"]) > 0)

    # Category assertions: top dimension and event search by category.
    cats = get(base, f"/api/v1/top?{window}&dimension=categories&limit=10")["rows"]
    expected_cats = {"Search Engines", "Streaming Media", "Social Networking",
                     "Information Technology", "Business", "Content Delivery Networks",
                     "Web-based Email"}
    found_cats = {row["key"] for row in cats}
    check("top categories include seeded categories",
          expected_cats.issubset(found_cats), f"found {found_cats}")
    check("top categories ranked",
          all(cats[i]["events"] >= cats[i + 1]["events"] for i in range(len(cats) - 1)))

    term = urllib.parse.quote("Streaming Media")
    found = get(base, f"/api/v1/events?{window}&q={term}&limit=50")["rows"]
    check("category search returns rows", len(found) > 0)
    check(
        "category search rows match",
        all(
            row.get("category") == "Streaming Media"
            for row in found
        ),
    )

    term = urllib.parse.quote("alice")
    found = get(base, f"/api/v1/events?{window}&q={term}&limit=50")["rows"]
    check("search returns rows", len(found) > 0)
    check(
        "search rows match",
        all(
            "alice"
            in " ".join(
                str(row.get(key) or "")
                for key in ("user", "domain", "url", "message", "threat", "rule", "src_ip")
            ).lower()
            for row in found
        ),
    )

    try:
        get(base, "/api/v1/top?dimension=not-a-dimension")
        check("invalid dimension rejected", False)
    except urllib.error.HTTPError as err:
        check("invalid dimension rejected", err.code == 400, str(err.code))

    with urllib.request.urlopen(base + "/", timeout=15) as response:
        html = response.read().decode("utf-8", "replace")
    check("dashboard HTML served", "Sophos Reporter" in html)

    with urllib.request.urlopen(base + "/healthz", timeout=15) as response:
        health = json.load(response)
    check("health endpoint ok", health.get("status") == "ok" and health.get("database") is True)

    print(f"\nseed: {result.stderr.strip()}")
    if failures:
        print(f"\n{len(failures)} check(s) failed: {failures}")
        return 1
    print("\nAll smoke checks passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
