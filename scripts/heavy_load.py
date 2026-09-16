#!/usr/bin/env python3
"""
Heavy load generator with unique subnets.
Generates ~500k unique IPs across diverse /24 subnets.
"""
import socket
import json
import time
import random
import threading
import urllib.request

IPC_HOST = "127.0.0.1"
IPC_PORT = 7890
BATCH_SIZE = 50       # events per frame
FRAME_INTERVAL = 0.01 # 100 frames/s → 5k events/s per sender
DURATION = 60         # seconds
NUM_SENDERS = 4       # parallel senders
SUBNET_BASE = 1       # starting subnet counter


def make_frame(batch_size, subnet_offset):
    """Build a single IPC frame with unique IPs."""
    events = []
    for i in range(batch_size):
        n = subnet_offset + i
        # Spread across many /24 subnets: use first 3 octets as unique ID
        # 1.0.0.x through 255.255.255.x gives ~16M possible subnets
        a = (n >> 16) & 0xFF
        b = (n >> 8) & 0xFF
        c = n & 0xFF
        d = random.randint(1, 254)
        ip = f"{a}.{b}.{c}.{d}"
        events.append({
            "ip": ip,
            "bytes": random.randint(64, 16000),
            "status_code": random.choice([200, 200, 200, 200, 301, 404, 500, 503]),
            "proto_fp": random.choice([1, 2, 3, 6, 17])
        })
    return json.dumps({"type": "report_connections", "events": events}) + "\n"


def sender_thread(thread_id, total_events, subnet_start, stats):
    """Send events in batches, tracking unique subnets."""
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    s.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
    s.connect((IPC_HOST, IPC_PORT))

    sent = 0
    subnet_counter = subnet_start
    start = time.monotonic()

    while sent < total_events:
        frame = make_frame(BATCH_SIZE, subnet_counter)
        try:
            s.sendall(frame.encode())
        except (BrokenPipeError, OSError):
            print(f"  Thread {thread_id}: reconnecting...")
            s.close()
            time.sleep(0.5)
            s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            s.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
            try:
                s.connect((IPC_HOST, IPC_PORT))
                continue
            except Exception:
                break

        sent += BATCH_SIZE
        subnet_counter += BATCH_SIZE
        stats['sent'] += BATCH_SIZE
        stats['subnets'] = subnet_counter

        time.sleep(FRAME_INTERVAL)

    s.close()
    elapsed = time.monotonic() - start
    stats['elapsed'] = elapsed
    stats['rate'] = sent / elapsed if elapsed > 0 else 0


def monitor_dashboard(duration):
    """Monitor dashboard for the duration."""
    start = time.monotonic()

    print(f"\n{'='*70}")
    print(f" Monitoring dashboard for {duration}s")
    print(f"{'='*70}")
    print(f"{'time':>5} {'ips_tracked':>12} {'ingested':>10} {'cpu':>6} {'subnet_ones':>11} {'hot_subnets':>11} {'health':>7}")
    print(f"{'-'*70}")

    for i in range(duration):
        time.sleep(1)
        try:
            snapshot = json.loads(urllib.request.urlopen(
                f"http://{IPC_HOST}:9999/api/snapshot", timeout=2
            ).read())

            print(f"  t+{i+1:3d}s "
                  f"{snapshot.get('ips_tracked', '?'):>12,} "
                  f"{snapshot.get('events_ingested', '?'):>10,} "
                  f"{snapshot.get('cpu_usage_pct', 0):>5.1f}% "
                  f"{snapshot.get('subnet_bitmap_ones', '?'):>11,} "
                  f"{snapshot.get('hot_subnets_count', '?'):>11,} "
                  f"{snapshot.get('is_healthy', '?'):>7}")
        except Exception as e:
            print(f"  t+{i+1:3d}s error: {e}")

    print(f"{'='*70}")


print(f"RamShield Heavy Load Generator — Unique Subnets")
print(f"=" * 70)
print(f"Target: ~{(BATCH_SIZE * (DURATION * 100 / FRAME_INTERVAL)) * NUM_SENDERS:,.0f} events across ~{DURATION * 100 / FRAME_INTERVAL * BATCH_SIZE * NUM_SENDERS:,.0f} unique IPs")
print(f"Batches: {BATCH_SIZE} events/frame, {FRAME_INTERVAL*1000:.0f}ms interval")
print(f"Senders: {NUM_SENDERS} threads")
print(f"Duration: {DURATION}s")
print(f"=" * 70)

# Stats collection
shared_stats = {'sent': 0, 'elapsed': 0, 'rate': 0, 'subnets': 0}

# Start monitor in background
monitor_thread = threading.Thread(target=monitor_dashboard, args=(DURATION,), daemon=True)
monitor_thread.start()

# Start sender threads — each gets unique subnet offset
threads = []
events_per_thread = (BATCH_SIZE * int(DURATION / FRAME_INTERVAL)) // NUM_SENDERS

for i in range(NUM_SENDERS):
    subnet_offset = i * events_per_thread + SUBNET_BASE
    t = threading.Thread(
        target=sender_thread,
        args=(i, events_per_thread, subnet_offset, shared_stats),
        daemon=True
    )
    threads.append(t)
    t.start()
    print(f"  Started sender thread {i} (subnets {subnet_offset:,}..{subnet_offset + events_per_thread:,})")

# Wait for completion
for t in threads:
    t.join()

# Final report
print(f"\n{'='*70}")
print(f"Load Generation Complete")
print(f"{'='*70}")
print(f"Total events sent: {shared_stats['sent']:,}")
print(f"Unique subnets targeted: ~{shared_stats['subnets']:,}")
print(f"Elapsed time: {shared_stats['elapsed']:.2f}s")
print(f"Overall rate: {shared_stats['rate']:,.0f} events/s")

# Get final snapshot
try:
    snapshot = json.loads(urllib.request.urlopen(
        f"http://{IPC_HOST}:9999/api/snapshot", timeout=2
    ).read())
    print(f"\nFinal Dashboard State:")
    print(f"  IPs tracked: {snapshot.get('ips_tracked', '?'):,}")
    print(f"  Events ingested: {snapshot.get('events_ingested', '?'):,}")
    print(f"  Subnet bitmap ones: {snapshot.get('subnet_bitmap_ones', '?'):,}")
    print(f"  Hot subnets: {snapshot.get('hot_subnets_count', '?'):,}")
    print(f"  Health: {snapshot.get('is_healthy', '?')}")
    print(f"  XDP active: {snapshot.get('xdp_active', '?')}")
except Exception as e:
    print(f"  Final snapshot error: {e}")

print(f"{'='*70}")
