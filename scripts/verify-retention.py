#!/usr/bin/env python3
"""Verify global retention preview, bearer auth, and persistent API behavior."""

from __future__ import annotations

import argparse
import json
import sys
import urllib.error
import urllib.request


def request_json(url: str, token: str | None = None, body: dict | None = None) -> dict:
    headers = {"Accept": "application/json"}
    data = None
    method = None
    if token is not None:
        headers["Authorization"] = f"Bearer {token}"
    if body is not None:
        headers["Content-Type"] = "application/json"
        data = json.dumps(body).encode()
        method = "PUT"
    request = urllib.request.Request(url, data=data, headers=headers, method=method)
    with urllib.request.urlopen(request, timeout=15) as response:
        return json.load(response)


def check(name: str, condition: bool, detail: str = "") -> None:
    print(("PASS  " if condition else "FAIL  ") + name + ("" if condition else f" [{detail}]"))
    if not condition:
        raise AssertionError(name)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base-url", required=True)
    parser.add_argument("--token", required=True)
    args = parser.parse_args()
    base = args.base_url.rstrip("/")
    url = base + "/api/v1/retention"

    try:
        preview = request_json(url)
        choices = preview["choices_hours"]
        original_hours = preview["current_hours"]
        check("preview is explicitly global", preview["global"] is True)
        check("preview includes database size", preview["database_bytes"] > 0)
        check("preview exposes allowed choices", choices == [6, 12, 24, 48, 72, 168], str(choices))
        check(
            "partition-based storage projection is available",
            preview["estimated_raw_bytes_per_hour"] is not None
            and all(option["estimated_database_bytes"] is not None for option in preview["options"]),
            json.dumps({
                "hourly_estimate": preview["estimated_raw_bytes_per_hour"],
                "raw_partition_bytes": preview["current_raw_partition_bytes"],
                "options": preview["options"],
            }),
        )
        check("admin token is configured", preview["admin_configured"] is True)

        try:
            request_json(url, body={"retention_hours": next(h for h in choices if h != original_hours)})
            check("unauthenticated update is rejected", False)
        except urllib.error.HTTPError as err:
            check("unauthenticated update is rejected", err.code == 401, str(err.code))

        changed_hours = next(hours for hours in choices if hours != original_hours)
        try:
            updated = request_json(url, args.token, {"retention_hours": changed_hours})
            check("authenticated update is accepted", updated["retention_hours"] == changed_hours)
            meta = request_json(base + "/api/v1/meta")
            check(
                "API reads the persisted global setting",
                meta["raw_retention_hours"] == changed_hours,
                str(meta["raw_retention_hours"]),
            )
            current = request_json(url)
            check("setting remains persisted", current["current_hours"] == changed_hours)
        finally:
            request_json(url, args.token, {"retention_hours": original_hours})

        check("original retention setting restored", request_json(url)["current_hours"] == original_hours)
    except (OSError, TimeoutError, ValueError, KeyError, AssertionError, urllib.error.URLError) as err:
        print(f"FAIL  retention verification failed: {err}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
