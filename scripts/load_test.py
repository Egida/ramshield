#!/usr/bin/env python3
"""Generate realistic load against RamShield IPC + HTTP endpoints."""
import socket
import json
import time
import random
import threading
import urllib.request

IPC_HOST = "127.0.0.1"
IPC_PORT = 7890
DASH_HOST = "127.0.0.1"
DASH_PORT = 9999

# Realistic source IP pools
ATTACKER_IPS = [f"10.{random.randint(1,250)}.{random.randint(1,250)}.{random.randint(1,254)}" for _ in range(80)]
BENIGN_IPS = [f"192.168.{random.randint(1,254)}.{random.randint(1,254)}" for _ in range(200)]
CGNAT_IPS = [f"100.{random.randint(64,127)}.{random.randint(0,255)}.{random.randint(1,254)}" for _ in range(50)]
ALL_IPS = ATTACKER_IPS * 3 + BENIGN_IPS + CGNAT_IPS

PROTO_FP = [1, 2, 3, 6, 17]
STATUS_CODES = [200, 200, 200, 200, 200, 301, 404, 500, 503]


def make_frame(batch_size, anomaly=False):
    """Build one IPC frame: {"type":"report_connections","events":[...]}"""
    events = []
    for _ in range(batch_size):
        src = random.choice(ATTACKER_IPS if anomaly else ALL_IPS)
        events.append({
            "ip": src,
            "bytes": random.randint(1400, 9000) if anomaly else random.randint(64, 8192),
            "status_code": random.choice([403, 403, 403, 500]) if anomaly else random.choice(STATUS_CODES),
            "proto_fp": random.choice([6, 17]) if anomaly else random.choice(PROTO_FP),
        })
    return json.dumps({"type": "report_connections", "events": events}) + "\n"


def ipc_sender(duration, batch_size, batch_rate, anomaly=False, name="ipc"):
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    s.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
    try:
        s.connect((IPC_HOST, IPC_PORT))
    except Exception as e:
        print(f"[{name}] FAILED: {e}")
        return 0

    sent = 0
    interval = 1.0 / batch_rate if batch_rate > 0 else 0
    start = time.monotonic()

    while time.monotonic() - start < duration:
        frame = make_frame(batch_size, anomaly=anomaly)
        try:
            s.sendall(frame.encode())
        except Exception:
            break
        sent += batch_size
        if interval > 0:
            time.sleep(interval)

    s.close()
    return sent


def http_load(duration, rps):
    urls = [
        f"http://{DASH_HOST}:{DASH_PORT}/api/snapshot",
        f"http://{DASH_HOST}:{DASH_PORT}/api/blocks/active",
    ]
    interval = 1.0 / rps if rps > 0 else 0
    start = time.monotonic()
    count = 0
    while time.monotonic() - start < duration:
        url = random.choice(urls)
        try:
            urllib.request.urlopen(url, timeout=2)
        except Exception:
            pass
        count += 1
        if interval > 0:
            time.sleep(interval)
    return count


def snapshot():
    try:
        d = json.loads(urllib.request.urlopen(f"http://{DASH_HOST}:{DASH_PORT}/api/snapshot", timeout=2).read())
        return {k: d.get(k) for k in ["events_ingested", "channel_depth", "proxy_l7_rps",
                "subnet_table_size", "cusum_accumulator", "is_healthy", "xdp_active"]}
    except Exception:
        return {}


print("=" * 60)
print("RamShield Load Test")
print("=" * 60)

# Phase 1: Warmup — 3 senders × 50 events × 20/s = ~3000 evt/s, 15s
print("\n[Phase 1] Warmup: ~3k evt/s, 15s")
threads = []
for i in range(3):
    t = threading.Thread(target=ipc_sender, args=(15, 50, 20), kwargs={"name": f"warmup-{i}"})
    threads.append(t)
    t.start()
for t in threads:
    t.join()
print(f"  Done. Snapshot: {snapshot()}")

# Phase 2: Attack — 3 attacker × 100 events × 10/s = ~3000 evt/s, 20s
print("\n[Phase 2] Attack burst: 3×attacker IPs, 20s")
threads = []
for i in range(3):
    t = threading.Thread(target=ipc_sender, args=(20, 100, 10), kwargs={"anomaly": True, "name": f"attack-{i}"})
    threads.append(t)
    t.start()
http_t = threading.Thread(target=http_load, args=(20, 100))
threads.append(http_t)
http_t.start()
for t in threads:
    t.join()
print(f"  Done. Snapshot: {snapshot()}")

# Phase 3: Cooldown
print("\n[Phase 3] Cooldown: ~1k evt/s, 10s")
ipc_sender(10, 50, 7)
print(f"  Done. Snapshot: {snapshot()}")

# Phase 4: Sustained
print("\n[Phase 4] Sustained: ~4.5k evt/s + 100 rps HTTP, 30s")
threads = []
for i in range(3):
    t = threading.Thread(target=ipc_sender, args=(30, 50, 30), kwargs={"name": f"sustained-{i}"})
    threads.append(t)
    t.start()
http_t = threading.Thread(target=http_load, args=(30, 100))
threads.append(http_t)
http_t.start()
for t in threads:
    t.join()
print(f"  Done. Snapshot: {snapshot()}")

print("\n" + "=" * 60)
print("Load test complete. Dashboard at http://127.0.0.1:9999")
print("=" * 60)
