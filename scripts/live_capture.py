#!/usr/bin/env python3
"""Simultaneous load + SSE capture — see rates live."""
import socket, json, time, random, threading, subprocess

IPC_HOST, IPC_PORT = "127.0.0.1", 7890
DASH_URL = "http://127.0.0.1:9999/api/stream"

ATTACKER_IPS = [f"10.{random.randint(1,250)}.{random.randint(1,250)}.{random.randint(1,254)}" for _ in range(80)]
BENIGN_IPS = [f"192.168.{random.randint(1,254)}.{random.randint(1,254)}" for _ in range(200)]
ALL_IPS = ATTACKER_IPS * 3 + BENIGN_IPS

def load_gen(dur, stop_event):
    """Send batches continuously for dur seconds."""
    s = socket.socket()
    s.connect((IPC_HOST, IPC_PORT))
    start = time.monotonic()
    count = 0
    while not stop_event.is_set() and time.monotonic() - start < dur:
        events = []
        for _ in range(50):
            src = random.choice(ALL_IPS)
            events.append({"ip": src, "bytes": random.randint(64, 8192),
                           "status_code": random.choice([200,200,200,404,500]),
                           "proto_fp": random.choice([1,2,3,6,17])})
        s.sendall(json.dumps({"type": "report_connections", "events": events}).encode() + b"\n")
        count += 50
        time.sleep(0.02)
    s.close()
    return count

print("Starting load (200 events × 50 batch/s = ~10k evt/s)...")
stop = threading.Event()
t = threading.Thread(target=load_gen, args=(20, stop))
t.start()

# Capture SSE at 3 timepoints during load
import urllib.request
for i in range(3):
    time.sleep(3)
    try:
        # Use subprocess to get one frame
        r = subprocess.run(["timeout", "1", "curl", "-sS", DASH_URL],
                           capture_output=True, text=True, timeout=3)
        for line in r.stdout.split("\n"):
            if line.startswith("data: "):
                d = json.loads(line[6:])
                rates = {
                    "ingest_rps": d.get("ipc", {}).get("ingest_rps", 0),
                    "rejected_rps": d.get("ipc", {}).get("rejected_rps", 0),
                    "promotions": d.get("detection", {}).get("promotions_rps", 0),
                    "blocks_det": d.get("detection", {}).get("blocks_detection_rps", 0),
                    "entropy": d.get("cgnat", {}).get("cgnat_entropy_score") or d.get("cgnat_entropy_score", 0),
                    "cusum": d.get("cusum_accumulator", 0),
                    "hot_subnets": d.get("detection", {}).get("hot_subnets_count", 0),
                    "ips_tracked": d.get("system", {}).get("ips_tracked", 0),
                    "wire_pass": d.get("xdp_wire_pass", 0),
                    "cpu": round(d.get("system", {}).get("cpu_usage_pct", 0), 1),
                    "rss_mb": d.get("system", {}).get("rss_mb", 0),
                    "health": d.get("health", {}).get("is_healthy"),
                }
                print(f"\n  t+{i*3+3}s: {json.dumps(rates)}")
                break
    except Exception as e:
        print(f"\n  t+{i*3+3}s: capture error: {e}")

stop.set()
t.join()

# Final capture after traffic stops
time.sleep(2)
r = subprocess.run(["timeout", "1", "curl", "-sS", DASH_URL], capture_output=True, text=True, timeout=3)
for line in r.stdout.split("\n"):
    if line.startswith("data: "):
        d = json.loads(line[6:])
        print(f"\n  Final (idle): ingest_rps={d.get('ipc',{}).get('ingest_rps',0)}, cusum={d.get('cusum_accumulator',0)}, "
              f"hot_subnets={d.get('detection',{}).get('hot_subnets_count',0)}, "
              f"wire_pass={d.get('xdp_wire_pass',0)}")
        break

print("\nDone.")
