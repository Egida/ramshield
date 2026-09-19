#!/usr/bin/env python3
"""Sparse /24 swarm stress: many hosts per /24, few events per host.

Targets the gatekeeper fix (commit 985091a): sparse subnets the OLD promote
gate cold-skipped entirely now promote via swarm_hint.

40 subnets x 60 hosts x 2 events = 4,800 events.
Each /24: 60 unique hosts (>= subnet_batch_threshold=50) and 120 events
(>= subnet_batch_min_events=100), so all three swarm_hint legs can arm.

Sequential connections: one socket per frame, open-send-close. The IPC server
closes request connections; parallel persistent senders hit BrokenPipe.
"""
import json
import socket
import time
import urllib.request

IPC = ("127.0.0.1", 7890)
DASH = "http://127.0.0.1:9999"
N_SUBNETS = 40
HOSTS = 60
EVENTS_PER_HOST = 2


def frame(subnet):
    events = []
    for h in range(1, HOSTS + 1):
        for e in range(EVENTS_PER_HOST):
            events.append({
                "ip": f"172.16.{subnet}.{h}",
                "bytes": 512 + e * 8,
                "status_code": 200,
                "proto_fp": 6,
            })
    return (json.dumps({"type": "report_connections", "events": events}) + "\n").encode()


def send(payload):
    s = socket.create_connection(IPC, timeout=5)
    try:
        s.sendall(payload)
        return s.recv(4096)
    finally:
        s.close()


def snap():
    with urllib.request.urlopen(f"{DASH}/api/snapshot", timeout=3) as r:
        return json.loads(r.read())


KEYS = ("events_ingested", "cold_skipped", "promotions", "blocks_applied",
        "ips_tracked", "batches_total", "is_healthy")


def counters():
    s = snap()
    return {k: s.get(k) for k in KEYS}


before = counters()
print(f"before: {json.dumps(before)}")

t0 = time.monotonic()
total = N_SUBNETS * HOSTS * EVENTS_PER_HOST
sent = 0
for sub in range(N_SUBNETS):
    reply = json.loads(send(frame(sub)))
    accepted = reply.get("accepted", 0)
    sent += accepted
    if accepted != HOSTS * EVENTS_PER_HOST:
        print(f"FAIL subnet {sub}: accepted {accepted}/{HOSTS * EVENTS_PER_HOST}")
        raise SystemExit(1)
elapsed = time.monotonic() - t0
print(f"sent {sent}/{total} events in {elapsed:.2f}s "
      f"({sent / elapsed:,.0f} events/s)")

# The subnet loop scans on a ~500ms tick; give it a few ticks to fire.
time.sleep(4)
after = counters()
print(f"after:  {json.dumps(after)}")

d_ingest = (after["events_ingested"] or 0) - (before["events_ingested"] or 0)
d_cold = (after["cold_skipped"] or 0) - (before["cold_skipped"] or 0)
d_promo = (after["promotions"] or 0) - (before["promotions"] or 0)
d_blocks = (after["blocks_applied"] or 0) - (before["blocks_applied"] or 0)

print(f"delta: ingested={d_ingest} cold_skipped={d_cold} "
      f"promotions={d_promo} blocks={d_blocks} ips_tracked={after['ips_tracked']}")

checks = [
    ("all events accepted", sent == total),
    ("events ingested", d_ingest > 0),
    ("swarm hosts promoted (ips_tracked >= 1000)", (after["ips_tracked"] or 0) >= 1000),
    ("cold_skipped < 50% of ingested", d_cold < d_ingest * 0.5),
    ("subnet dual gate fired (blocks >= 1)", d_blocks >= 1),
    ("healthy", after["is_healthy"] is True),
]
failed = 0
for name, ok in checks:
    print(f"  [{'PASS' if ok else 'FAIL'}] {name}")
    failed += 0 if ok else 1
print(f"{'ALL CHECKS PASSED' if not failed else f'{failed} CHECK(S) FAILED'}")
raise SystemExit(1 if failed else 0)