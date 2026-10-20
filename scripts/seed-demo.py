#!/usr/bin/env python3
"""Send realistic Sophos Firewall (SFOS) syslog messages to a collector.

Useful for trying the dashboard without a firewall, and as an end-to-end smoke
test: the JSON summary printed on stdout describes exactly what the collector
should report for the generated window.

    python3 scripts/seed-demo.py --host 127.0.0.1 --port 5514
"""

from __future__ import annotations

import argparse
import json
import random
import socket
import sys
import time
from datetime import datetime, timedelta, timezone

DEVICE_NAME = "DEMO-XGS2100"
DEVICE_ID = "DEMO000000000"

WEB_USERS = [
    ("alice", "Staff"),
    ("bob", "Staff"),
    ("carol", "Finance"),
    ("dave", "Engineering"),
    ("eve", "Engineering"),
]

DESTINATIONS = [
    # domain, category, category type, weight
    ("www.google.com", "Search Engines", "Productive", 26),
    ("www.youtube.com", "Streaming Media", "Unproductive", 22),
    ("www.facebook.com", "Social Networking", "Unproductive", 14),
    ("github.com", "Information Technology", "Productive", 12),
    ("www.example.org", "Business", "Productive", 10),
    ("cdn.jsdelivr.net", "Content Delivery Networks", "Neutral", 8),
    ("slack.com", "Web-based Email", "Productive", 8),
]

THREATS = [
    ("EICAR-Test-File", "Anti-Virus", "SMTP"),
    ("Trojan.Generic.12345", "Anti-Virus", "HTTP"),
    ("Mal/HTMLGen-A", "Intrusion Prevention", "IPS"),
    ("CryptoLocker", "ATP", "ATP"),
]

FIREWALL_RULES = [
    ("Allow LAN to WAN", 3),
    ("Guest Wi-Fi", 7),
    ("Block IoT", 9),
]


def q(value: str) -> str:
    return '"' + value.replace("\\", "\\\\").replace('"', '\\"') + '"'


def envelope(ts: datetime, log_id: str, log_type: str, component: str, subtype: str,
             status: str, priority: str, body: str) -> str:
    return (
        f'device="SFW" date={ts:%Y-%m-%d} time={ts:%H:%M:%S} timezone="UTC" '
        f'device_name={q(DEVICE_NAME)} device_id={DEVICE_ID} log_id={log_id} '
        f'log_type={q(log_type)} log_component={q(component)} log_subtype={q(subtype)} '
        f'status={q(status)} priority={q(priority)} {body}'
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=5514)
    parser.add_argument("--web", type=int, default=1200)
    parser.add_argument("--firewall", type=int, default=300)
    parser.add_argument("--threats", type=int, default=80)
    parser.add_argument("--auth", type=int, default=60)
    parser.add_argument("--vpn", type=int, default=20)
    parser.add_argument("--hours", type=int, default=23, help="spread events over the last N hours")
    parser.add_argument("--seed", type=int, default=42)
    parser.add_argument("--udp", action="store_true", help="send datagrams instead of a TCP stream")
    parser.add_argument("--summary-file", help="also write the expected summary to this path")
    parser.add_argument("--no-noise", action="store_true", help="skip the unparseable control lines")
    parser.add_argument("--yes", action="store_true", help="skip the write confirmation")
    args = parser.parse_args()

    if not args.yes:
        print(
            "This sends synthetic events into the collector your firewall also uses.\n"
            "They are tagged as DEMO-XGS2100; remove them with scripts/reset-data.sh --demo-only.\n"
            "Re-run with --yes to continue.",
            file=sys.stderr,
        )
        return 2

    rng = random.Random(args.seed)
    now = datetime.now(timezone.utc).replace(microsecond=0)
    window = max(60, args.hours * 3600)

    def timestamp() -> datetime:
        return now - timedelta(seconds=rng.randint(60, window))

    lines: list[str] = []
    summary = {
        "events": 0,
        "web_requests": 0,
        "allowed": 0,
        "blocked": 0,
        "bytes_up": 0,
        "bytes_down": 0,
        "threats": 0,
        "users": set(),
        "domains": set(),
    }

    def record(user: str | None, domain: str | None, action: str | None,
               up: int = 0, down: int = 0, web: bool = False, threat: bool = False) -> None:
        summary["events"] += 1
        if web:
            summary["web_requests"] += 1
        if action == "allowed":
            summary["allowed"] += 1
        if action == "blocked":
            summary["blocked"] += 1
        summary["bytes_up"] += up
        summary["bytes_down"] += down
        if threat:
            summary["threats"] += 1
        if user:
            summary["users"].add(user)
        if domain:
            summary["domains"].add(domain)

    destinations = [row for row in DESTINATIONS for _ in range(row[3])]

    for i in range(args.web):
        ts = timestamp()
        user, group = rng.choice(WEB_USERS)
        domain, category, category_type, _ = rng.choice(destinations)
        blocked = rng.random() < 0.18
        up = rng.randint(200, 4000)
        down = 0 if blocked else rng.randint(2_000, 4_000_000)
        path = rng.choice(["/", "/search?q=sophos+firewall", "/watch?v=dQw4w9WgXcQ", "/docs/intro"])
        line = envelope(
            ts,
            f"031006209{i % 1000:03d}",
            "Content Filtering",
            "HTTP",
            "Denied" if blocked else "Allowed",
            "Denied" if blocked else "Allowed",
            "Warning" if blocked else "Information",
            f"fw_rule_id=3 fw_rule_name={q('Allow LAN to WAN')} "
            f"user_name={q(user)} user_gp={q(group)} "
            f"src_ip=192.168.10.{rng.randint(10, 90)} "
            f"dst_ip=203.0.113.{rng.randint(10, 250)} protocol={q('TCP')} "
            f"src_port={rng.randint(1024, 65000)} dst_port=443 "
            f"domain={q(domain)} category={q(category)} category_type={q(category_type)} "
            f"url={q('https://' + domain + path)} http_method={q('GET')} "
            f"http_status={403 if blocked else 200} mime_type={q('text/html')} "
            f"sent_bytes={up} recv_bytes={down}",
        )
        lines.append(line)
        record(user, domain, "blocked" if blocked else "allowed", up, down, web=True)

    for i in range(args.firewall):
        ts = timestamp()
        user = rng.choice([name for name, _ in WEB_USERS] + ["svc-backup"])
        rule, rule_id = rng.choice(FIREWALL_RULES)
        blocked = rng.random() < 0.1
        up = rng.randint(500, 50_000)
        down = 0 if blocked else rng.randint(1_000, 2_000_000)
        line = envelope(
            ts,
            f"010101600{i % 1000:03d}",
            "Firewall",
            "Firewall Rule",
            "Denied" if blocked else "Allowed",
            "Deny" if blocked else "Allow",
            "Warning" if blocked else "Information",
            f"fw_rule_id={rule_id} fw_rule_name={q(rule)} user_name={q(user)} "
            f"src_ip=192.168.10.{rng.randint(10, 90)} dst_ip=198.51.100.{rng.randint(1, 250)} "
            f"src_port={rng.randint(1024, 65000)} dst_port={rng.choice([80, 443, 53, 123])} "
            f"protocol={q('TCP')} sent_bytes={up} recv_bytes={down} status_code=0",
        )
        lines.append(line)
        record(user, None, "blocked" if blocked else "allowed", up, down)

    for i in range(args.threats):
        ts = timestamp()
        threat, log_type, component = rng.choice(THREATS)
        user = rng.choice([name for name, _ in WEB_USERS] + [None])
        line = envelope(
            ts,
            f"054002616{i % 1000:03d}",
            log_type,
            component,
            "Virus" if log_type == "Anti-Virus" else "Signature",
            "",
            "Warning",
            f"user_name={q(user) if user else q('')} "
            f"src_ip=192.168.10.{rng.randint(10, 90)} dst_ip=203.0.113.{rng.randint(10, 250)} "
            f"protocol={q('TCP')} src_port={rng.randint(1024, 65000)} dst_port=80 "
            f"threatname={q(threat)} filename={q('payload.bin')} message={q('Threat blocked')}",
        )
        lines.append(line)
        record(user, None, None, threat=True)

    for i in range(args.auth):
        ts = timestamp()
        user = rng.choice(["admin", "it-admin"])
        line = envelope(
            ts,
            f"062009617{i % 1000:03d}",
            "Event",
            "FirewallAuthentication",
            "Authentication",
            "",
            "Information",
            f"user_name={q(user)} src_ip=192.168.10.{rng.randint(10, 90)} "
            f"message={q('User logged in successfully through Local authentication mechanism')}",
        )
        lines.append(line)
        record(user, None, None)

    for i in range(args.vpn):
        ts = timestamp()
        user = rng.choice(["bob", "carol", "frank"])
        line = envelope(
            ts,
            f"041303617{i % 1000:03d}",
            "VPN",
            "SSL VPN",
            "Connected",
            "Success",
            "Information",
            f"user_name={q(user)} vpn_user={q(user)} tunnel_name={q('SSLVPNSite')} "
            f"src_ip=203.0.113.{rng.randint(10, 250)} message={q('User connected')}",
        )
        lines.append(line)
        record(user, None, None)

    # A valid event wrapped in an RFC3164 header, as sent through a syslog relay.
    ts = timestamp()
    wrapped = (
        f"<134>{ts:%b %d %H:%M:%S} {DEVICE_NAME} SFOS: "
        + envelope(
            ts,
            "010101600999",
            "Firewall",
            "Firewall Rule",
            "Allowed",
            "Allow",
            "Information",
            f"fw_rule_id=3 fw_rule_name={q('Allow LAN to WAN')} user_name={q('alice')} "
            f"src_ip=192.168.10.20 dst_ip=198.51.100.20 src_port=55000 dst_port=443 "
            f"protocol={q('TCP')} sent_bytes=1000 recv_bytes=2000",
        )
    )
    lines.append(wrapped)
    record("alice", None, "allowed", 1000, 2000)

    rng.shuffle(lines)

    noise = [
        "<134>Oct 17 10:00:00 otherhost CEF:0|Vendor|Product|1.0|100|event|5|src=10.0.0.1",
        'device="UTM" date=2020-01-01 time=00:00:00 log_id=010101600001 log_type="Firewall"',
        "not a syslog line at all",
    ]

    payload = lines + ([] if args.no_noise else noise)
    body = ("\n".join(payload) + "\n").encode("utf-8")

    start = time.time()
    if args.udp:
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
            for line in payload:
                sock.sendto((line + "\n").encode("utf-8"), (args.host, args.port))
    else:
        with socket.create_connection((args.host, args.port), timeout=10) as conn:
            conn.sendall(body)
    elapsed = time.time() - start

    summary["users"] = len(summary["users"])
    summary["domains"] = len(summary["domains"])
    summary["accepted_lines"] = len(lines)
    summary["rejected_lines"] = 0 if args.no_noise else len(noise)

    print(json.dumps(summary, indent=2))
    print(
        f"sent {len(payload)} lines to {args.host}:{args.port} in {elapsed:.2f}s",
        file=sys.stderr,
    )
    if args.summary_file:
        with open(args.summary_file, "w", encoding="utf-8") as handle:
            json.dump(summary, handle, indent=2)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
