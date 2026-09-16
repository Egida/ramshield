#!/usr/bin/env python3
"""
Block producer: concentrated traffic from few IPs to trigger detection.
Each attacker gets 50+ events → crosses promote_min_events=8 threshold.
"""
import socket
import json
import time
import random
import threading
import urllib.request

IPC_HOST = "127.0.0.1"
IPC_PORT = 7890
DASH_URL = "http://127.0.0.1:9999"


def make_frame(events):
    return json.dumps({"type": "report_connections", "events": events}) + "\n"


def sender(s, events):
    """Send a batch, reconnect on failure."""
    for attempt in range(3):
        try:
            s.sendall(make_frame(events).encode())
            return s
        except (BrokenPipeError, OSError):
            s.close()
            time.sleep(0.1 * (attempt + 1))
            s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            s.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
            s.connect((IPC_HOST, IPC_PORT))
    return s


def snapshot():
    try:
        return json.loads(urllib.request.urlopen(f"{DASH_URL}/api/snapshot", timeout=2).read())
    except:
        return {}


def blocks():
    try:
        return json.loads(urllib.request.urlopen(f"{DASH_URL}/api/blocks/active", timeout=2).read())
    except:
        return []


print("Block Producer — generate blocks for quarantine feed")
print("=" * 70)

# Phase 1: benign background (30s, 1000 diverse IPs)
print("\n[Phase 1] Background: 1000 diverse IPs, 30s")
s = socket.socket(socket.AF_INET, socket.TCP_NODELAY, 1) if False else socket.socket()
s.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
s.connect((IPC_HOST, IPC_PORT))

bg_thread_stop = threading.Event()
def bg_traffic():
    n = 0
    while not bg_thread_stop.is_set():
        events = []
        for i in range(50):
            a = (n >> 16) & 0xFF
            b = (n >> 8) & 0xFF
            c = n & 0xFF
            events.append({
                "ip": f"{a}.{b}.{c}.{random.randint(1,254)}",
                "bytes": random.randint(64, 4096),
                "status_code": random.choice([200, 200, 200, 301]),
                "proto_fp": 6
            })
            n += 1
        s = sender(s, events)
        time.sleep(0.05)

bt = threading.Thread(target=bg_traffic, daemon=True)
bt.start()
time.sleep(30)
print(f"  Ingested: {snapshot().get('events_ingested', '?')}")

# Phase 2: attack bursts — 20 attacker IPs, 100 events each = triggers block
print("\n[Phase 2] Attack bursts: 20 IPs × 100 events")
for burst_round in range(5):
    attacker_ips = [f"192.168.200.{i}" for i in range(1, 21)]  # all in same /24
    events = []
    for ip in attacker_ips:
        for j in range(100):
            events.append({
                "ip": ip,
                "bytes": random.randint(1200, 9000),
                "status_code": random.choice([403, 403, 403, 500]),
                "proto_fp": random.choice([6, 17])
            })
            if len(events) >= 50:
                s = sender(s, events)
                events = []
                time.sleep(0.01)
    if events:
        s = sender(s, events)
    time.sleep(1)

    snap = snapshot()
    blk = blocks()
    print(f"  Round {burst_round+1}: ips_tracked={snap.get('ips_tracked',0)}, "
          f"cusum={snap.get('cusum_accumulator',0)}, "
          f"blocked_total={snap.get('blocked_total',0)}, "
          f"active_blocks={len(blk)}")

# Phase 3: keep traffic flowing + monitor blocks
print("\n[Phase 3] Sustained traffic + monitor quarantine feed (60s)")
for i in range(60):
    # Mix of normal and attack
    events = []
    for j in range(30):
        if random.random() < 0.3:
            ip = f"10.99.{random.randint(1,200)}.{random.randint(1,254)}"
            events.append({"ip": ip, "bytes": random.randint(1200, 9000),
                          "status_code": random.choice([403, 500]), "proto_fp": 6})
        else:
            ip = f"172.16.{random.randint(1,250)}.{random.randint(1,254)}"
            events.append({"ip": ip, "bytes": random.randint(64, 4096),
                          "status_code": random.choice([200, 200, 301]), "proto_fp": 6})
    s = sender(s, events)
    time.sleep(0.05)

    if i % 10 == 0:
        snap = snapshot()
        blk = blocks()
        print(f"  t+{i+1}s: ingested={snap.get('events_ingested',0)}, "
              f"blocked={snap.get('blocked_total',0)}, "
              f"active_blocks_api={len(blk)}, "
              f"subnet_ones={snap.get('subnet_bitmap_ones',0)}, "
              f"hot_subnets={snap.get('hot_subnets_count',0)}")

bg_thread_stop.set()
s.close()

# Final state
snap = snapshot()
blk = blocks()
print(f"\n{'='*70}")
print(f"Final State:")
print(f"  Events ingested: {snap.get('events_ingested', 0):,}")
print(f"  IPs tracked: {snap.get('ips_tracked', 0):,}")
print(f"  Blocked total: {snap.get('blocked_total', 0)}")
print(f"  Active blocks API: {len(blk)}")
if blk:
    print(f"  Sample blocks: {[b['ip'] for b in blk[:5]]}")
print(f"{'='*70}")
