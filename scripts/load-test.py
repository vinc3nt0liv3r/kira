#!/usr/bin/env python3
"""Load test: sustained ingestion while monitoring dashboard response times.

Usage:
    python3 scripts/load-test.py --base-url http://127.0.0.1:8080 \
        --host 127.0.0.1 --port 5514 --duration 60
"""

from __future__ import annotations

import argparse
import json
import statistics
import subprocess
import sys
import time
import urllib.request


def get_json(url: str, timeout: int = 10):
    with urllib.request.urlopen(url, timeout=timeout) as resp:
        return json.load(resp)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base-url", default="http://127.0.0.1:8080")
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=5514)
    parser.add_argument("--duration", type=int, default=60, help="seconds to run")
    parser.add_argument("--rate", type=int, default=500, help="events per second target")
    parser.add_argument("--metrics-url", default="http://127.0.0.1:8081/metrics")
    args = parser.parse_args()

    base = args.base_url.rstrip("/")
    print(f"Starting load test: {args.rate} events/sec for {args.duration}s")
    print(f"Base URL: {base}, Metrics: {args.metrics_url}")
    
    # Start generating events in background
    seed = "/home/zorin/Documents/kira/scripts/seed-demo.py"
    total_events = args.rate * args.duration
    # Generate in batches
    batches = max(1, args.duration // 5)
    events_per_batch = total_events // batches
    
    proc = subprocess.Popen(
        [sys.executable, seed, "--yes", "--host", args.host, "--port", str(args.port),
         "--web", str(events_per_batch * 4), "--firewall", str(events_per_batch),
         "--threats", str(events_per_batch // 10), "--auth", str(events_per_batch // 20),
         "--vpn", str(events_per_batch // 30), "--hours", "1"],
        stdout=subprocess.PIPE, stderr=subprocess.PIPE
    )
    
    latencies = []
    errors = 0
    start = time.time()
    
    while time.time() - start < args.duration:
        t0 = time.time()
        try:
            get_json(f"{base}/api/v1/overview?from=1h", timeout=15)
            latencies.append((time.time() - t0) * 1000)
        except Exception as e:
            errors += 1
            print(f"  ERROR: {e}", file=sys.stderr)
        try:
            m = get_json(args.metrics_url, timeout=5)
            qd = m.get("queue_depth", 0)
            qc = m.get("queue_capacity", 1)
            util = m.get("queue_utilization_percent", 0)
            dropped = m.get("dropped", 0)
            if util > 50:
                print(f"  WARN: queue {qd}/{qc} ({util:.1f}%), dropped={dropped}")
        except Exception:
            pass
        time.sleep(1)
    
    proc.wait(timeout=10)
    
    print("\n=== Results ===")
    print(f"Duration: {time.time() - start:.1f}s")
    print(f"Requests: {len(latencies) + errors}")
    print(f"Errors: {errors}")
    if latencies:
        print(f"Latency p50: {statistics.median(latencies):.1f}ms")
        print(f"Latency p95: {sorted(latencies)[int(len(latencies)*0.95)]:.1f}ms")
        print(f"Latency p99: {sorted(latencies)[int(len(latencies)*0.99)]:.1f}ms")
        print(f"Latency max: {max(latencies):.1f}ms")
    
    try:
        m = get_json(args.metrics_url)
        print(f"\nMetrics: written={m.get('written')}, dropped={m.get('dropped')}, db_errors={m.get('db_errors')}")
        print(f"Queue high water: {m.get('queue_high_water')}/{m.get('queue_capacity')}")
    except Exception as e:
        print(f"Could not fetch metrics: {e}")
    
    if errors > 0 or (latencies and sorted(latencies)[int(len(latencies)*0.95)] > 2000):
        print("\nFAIL: Performance thresholds exceeded")
        return 1
    print("\nPASS: Load test completed within thresholds")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
