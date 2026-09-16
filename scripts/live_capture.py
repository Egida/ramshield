#!/usr/bin/env python3
"""Simultaneous load + SSE capture — see rates live.

Reads the current rev2 SSE schema from /api/stream:
  top-level: xdp, ipc, detection, forecasting, cgnat, analytics, mesh, system,
             health, durability, pipeline, ts, ts_ms
  nested:    ipc.{ingest_total, rejected_total, shed_total, active_connections,
             ring_depth, ring_capacity}
             detection.modules[].detail.*  (IPC/Detection/Forecasting/CGNAT/Analytics/Mesh/Storage)
             system.{cpu_pct, rss_mb, ram_pct, ips_tracked, store_ram_bytes, store_ram_limit_mb, total_ram_mb}
             durability.{wal_lsn, pending_expirations}
             pipeline[]  (6 stages: wire/xdp/ipc/cold/detection/enforcement)
"""
import os, socket, json, time, random, threading, urllib.request

IPC_HOST = os.environ.get("RAMSHIELD_TEST_HOST", "127.0.0.1")
IPC_PORT = int(os.environ.get("RAMSHIELD_TEST_IPC_PORT", "7890"))
DASH_URL = os.environ.get("RAMSHIELD_TEST_DASH_URL", "http://127.0.0.1:9999/api/stream")

ATTACKER_IPS = [f"10.{random.randint(1,250)}.{random.randint(1,250)}.{random.randint(1,254)}"
                for _ in range(80)]
BENIGN_IPS = [f"192.168.{random.randint(1,254)}.{random.randint(1,254)}"
              for _ in range(200)]
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
                           "status_code": random.choice([200, 200, 200, 404, 500]),
                           "proto_fp": random.choice([1, 2, 3, 6, 17])})
        try:
            s.sendall(json.dumps({"type": "report_connections", "events": events}).encode() + b"\n")
        except (BrokenPipeError, ConnectionResetError):
            break
        count += 50
        time.sleep(0.02)
    s.close()
    return count


def fetch_sse_frame(timeout=2):
    """Read one SSE frame from /api/stream using Python HTTP (no curl hang)."""
    try:
        req = urllib.request.Request(DASH_URL + "/api/stream")
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            buf = b""
            deadline = time.monotonic() + timeout
            while time.monotonic() < deadline:
                chunk = resp.read(4096)
                if not chunk:
                    break
                buf += chunk
                for line in buf.split(b"\n"):
                    if line.startswith(b"data: "):
                        return json.loads(line[6:])
    except Exception:
        pass
    return None


def summarize(d):
    """Extract dashboard-relevant fields from a rev2 SSE frame."""
    if not d:
        return {"error": "no frame"}
    ipc = d.get("ipc", {})
    det = {m["label"]: m.get("detail", {}) for m in d.get("detection", {}).get("modules", [])}
    sys_ = d.get("system", {})
    dur = d.get("durability", {})
    return {
        "ingest_total": ipc.get("ingest_total", 0),
        "rejected_total": ipc.get("rejected_total", 0),
        "shed_total": ipc.get("shed_total", 0),
        "active_conns": ipc.get("active_connections", 0),
        "ring_depth": ipc.get("ring_depth", 0),
        "ring_cap": ipc.get("ring_capacity", 0),
        "detection_ingested": det.get("Detection", {}).get("ingested", 0),
        "detection_promoted": det.get("Detection", {}).get("promotions", 0),
        "detection_cold": det.get("Detection", {}).get("cold_skipped", 0),
        "detection_blocks": det.get("Detection", {}).get("blocks", 0),
        "hw_rps": det.get("Forecasting", {}).get("hw_rps", 0),
        "hw_zscore": det.get("Forecasting", {}).get("hw_zscore", 0),
        "entropy": det.get("Forecasting", {}).get("entropy", 0),
        "cg_allow": det.get("CGNAT", {}).get("tier_allow", 0),
        "cg_challenge": det.get("CGNAT", {}).get("tier_challenge", 0),
        "cg_powdrop": det.get("CGNAT", {}).get("tier_powdrop", 0),
        "cg_block": det.get("CGNAT", {}).get("tier_block", 0),
        "hll_inserts": det.get("Analytics", {}).get("hll_inserts", 0),
        "cms_increments": det.get("Analytics", {}).get("cms_increments", 0),
        "record_bans": det.get("Mesh", {}).get("record_bans", 0),
        "record_unbans": det.get("Mesh", {}).get("record_unbans", 0),
        "cpu_pct": round(sys_.get("cpu_pct", 0), 1),
        "rss_mb": sys_.get("rss_mb", 0),
        "ram_pct": round(sys_.get("ram_pct", 0), 1),
        "ips_tracked": sys_.get("ips_tracked", 0),
        "wal_lsn": dur.get("wal_lsn", 0),
        "pending_exp": dur.get("pending_expirations", 0),
        "health": d.get("health", {}).get("is_healthy"),
        "xdp_active": d.get("xdp", {}).get("active"),
        "pipeline_stages": len(d.get("pipeline", [])),
    }


print("Starting load (50 events × 50 batch/s = ~2.5k evt/s)...")
stop = threading.Event()
t = threading.Thread(target=load_gen, args=(20, stop))
t.start()

# Capture SSE at 3 timepoints during load
for i in range(3):
    time.sleep(3)
    d = fetch_sse_frame()
    print(f"\n  t+{i*3+3}s: {json.dumps(summarize(d))}")

stop.set()
t.join()

# Final capture after traffic stops
time.sleep(2)
d = fetch_sse_frame()
print(f"\n  Final (idle): {json.dumps(summarize(d))}")
print("\nDone.")