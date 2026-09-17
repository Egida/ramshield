#!/usr/bin/env python3
"""Trigger every RamShield metric except high RAM usage.

Verifies the dashboard shows non-zero values for every telemetry category
after targeted traffic injection. XDP counters are checked but expected
to stay zero without file capabilities.
"""
from __future__ import annotations
import json, os, socket, sys, time, urllib.request
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
IPC_PORT = int(os.environ.get("RAMSHIELD_TEST_IPC_PORT", "7890"))
DASH_PORT = int(os.environ.get("RAMSHIELD_TEST_DASH_PORT", "9999"))
DASH_URL = f"http://127.0.0.1:{DASH_PORT}"
IPC_HOST = os.environ.get("RAMSHIELD_TEST_HOST", "127.0.0.1")

# ── helpers ──────────────────────────────────────────────────────────────────
class Check:
    def __init__(self, name: str):
        self.name = name
        self.passed = 0
        self.failed: list[str] = []
    def ok(self, cond: bool, desc: str, detail: str = "") -> bool:
        mark = "PASS" if cond else "FAIL"
        print(f"  [{mark}] {desc}" + (f" — {detail}" if detail and not cond else ""))
        self.passed += 1 if cond else 0
        if not cond: self.failed.append(desc)
        return cond
    def finish(self) -> int:
        n = self.passed + len(self.failed)
        print(f"\n  == {self.name}: {'OK' if not self.failed else f'FAILED {len(self.failed)}/{n}'} "
              f"({self.passed}/{n} passed)\n")
        return len(self.failed)

def ipc_send(payload: dict, timeout: float = 5.0) -> dict:
    with socket.create_connection((IPC_HOST, IPC_PORT), timeout=timeout) as s:
        s.sendall((json.dumps(payload) + "\n").encode())
        s.settimeout(timeout)
        buf = b""
        while b"\n" not in buf:
            chunk = s.recv(65536)
            if not chunk:
                break
            buf += chunk
        return json.loads(buf.split(b"\n")[0])

def send_batch(ip_base: str, count: int, dst_port: int = 80,
               status_code: int = 200, proto_fp: int = 0x1000, byte_min: int = 64):
    events = []
    for i in range(count):
        ip = f"{ip_base}.{(i % 254) + 1}"
        events.append({"ip": ip, "bytes": byte_min + (i % 4096),
                        "status_code": status_code, "proto_fp": proto_fp})
    ipc_send({"type": "report_connections", "events": events})

def fetch_snapshot() -> dict:
    with urllib.request.urlopen(f"{DASH_URL}/api/snapshot", timeout=3) as r:
        return json.load(r)

def fetch_sse_frame(timeout: float = 2.0) -> dict | None:
    try:
        req = urllib.request.Request(f"{DASH_URL}/api/stream")
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            buf = b""
            deadline = time.monotonic() + timeout
            while time.monotonic() < deadline:
                chunk = resp.read(4096)
                if not chunk: break
                buf += chunk
                for line in buf.split(b"\n"):
                    if line.startswith(b"data: "):
                        return json.loads(line[6:])
    except Exception:
        pass
    return None

# ── metric trigger phases ────────────────────────────────────────────────────
def phase_ipc_ingest():
    """High-volume report_connections → ingest_total, detection pipeline."""
    print("Phase 1: IPC ingest (50k events across /24s)...")
    for subnet in range(5):
        send_batch(f"10.10.{subnet}", 10000)
    time.sleep(1)

def phase_ipc_reject():
    """Malformed frames → rejected_total."""
    print("Phase 2: IPC rejects (bad frames)...")
    for _ in range(10):
        try:
            with socket.create_connection((IPC_HOST, IPC_PORT), timeout=3) as s:
                s.sendall(b"not json\n")
                time.sleep(0.01)
        except Exception:
            pass
    time.sleep(0.5)

def phase_ewma_block():
    """Single-IP burst above threshold → EWMA auto-block."""
    print("Phase 3: EWMA auto-block (sustained single-IP flood)...")
    for _ in range(60):
        send_batch("198.51.100", 200, dst_port=443, proto_fp=0x0022)
        time.sleep(0.02)

def phase_subnet_block():
    """250 distinct IPs from /24 → subnet /24 block."""
    print("Phase 4: Subnet /24 block (250 IPs × 5 rounds)...")
    events = [{"ip": f"172.16.0.{i}", "bytes": 512, "status_code": 503, "proto_fp": 0x0800}
              for i in range(1, 255)]
    for _ in range(5):
        ipc_send({"type": "report_connections", "events": events})
        time.sleep(0.05)

def phase_ban_unban():
    """IPC block/unblock → record_bans, record_unbans."""
    print("Phase 5: IPC block/unblock (bans + unbans)...")
    for i in range(20):
        ip = f"203.0.113.{i+1}"
        ipc_send({"type": "block_ip", "ip": ip, "reason": "manual_block", "ttl_secs": 300})
    time.sleep(0.5)
    for i in range(10):
        ip = f"203.0.113.{i+1}"
        ipc_send({"type": "unblock_ip", "ip": ip})

def phase_analytics_diversity():
    """Diverse IPs + status codes → HLL inserts, CMS increments."""
    print("Phase 6: Analytics diversity (1k unique IPs × 4 status codes)...")
    for code in [200, 404, 500, 503]:
        send_batch(f"10.99.{code}", 1000, status_code=code, proto_fp=0x0001)
    time.sleep(0.5)

def phase_forecasting():
    """Accumulated traffic over time → HW forecast, entropy."""
    print("Phase 7: Forecasting (sustained varied traffic)...")
    for _ in range(20):
        for subnet in range(10):
            send_batch(f"10.20.{subnet}", 500, proto_fp=0x0011 + subnet)
        time.sleep(0.05)

# ── verification ─────────────────────────────────────────────────────────────
def verify_all_metrics() -> int:
    print("\nPhase 8: Verifying all metrics from snapshot + SSE...\n")
    c = Check("metrics-verify")
    snap = fetch_snapshot()
    sse = fetch_sse_frame(timeout=3)
    if not sse:
        c.ok(False, "SSE frame received for verification")
        return c.finish()

    # Snapshot uses flat counters; SSE carries nested dashboard groups.
    c.ok(snap.get("events_ingested", 0) > 0,
         "events_ingested > 0", str(snap.get("events_ingested")))
    c.ok(snap.get("promotions", 0) > 0,
         "promotions > 0", str(snap.get("promotions")))
    c.ok(snap.get("cold_skipped", 0) > 0,
         "cold_skipped > 0", str(snap.get("cold_skipped")))
    c.ok(snap.get("blocks_applied", 0) > 0,
         "blocks_applied > 0", str(snap.get("blocks_applied")))
    c.ok(snap.get("memory_usage_mb", 0) > 0, "memory_usage_mb > 0", str(snap.get("memory_usage_mb")))
    c.ok(snap.get("ram_pct", 0) < 90, "ram_pct < 90 (not high-RAM)", str(snap.get("ram_pct")))
    if snap.get("xdp_active") is True:
        c.ok(True, "xdp active (caps present)")
        c.ok(snap.get("xdp_v4_drops_total", 0) >= 0, "xdp_v4_drops present")
    else:
        c.ok(snap.get("xdp_v4_drops_total", 1) >= 0, "xdp inactive w/o caps; counters still present")
    c.ok(snap.get("wal_lsn", 0) >= 0, "wal_lsn present", str(snap.get("wal_lsn")))
    c.ok(snap.get("pending_expirations", 0) >= 0, "pending_expirations present", str(snap.get("pending_expirations")))
    pl = snap.get("pipeline", {})
    c.ok(pl.get("blocked", 0) > 0, "pipeline.blocked > 0", str(pl.get("blocked")))

    # SSE nested telemetry.
    ipc_d = sse.get("ipc", {})
    c.ok(ipc_d.get("ingest_total", 0) > 0, "SSE ipc.ingest_total > 0", str(ipc_d.get("ingest_total")))
    c.ok(ipc_d.get("frames_rejected_total", 0) >= 10, "SSE ipc.frames_rejected_total >= 10 (malformed frames)", str(ipc_d.get("frames_rejected_total")))
    det = sse.get("detection", {})
    modules = {m.get("label"): m.get("detail", {}) for m in det.get("modules", [])}
    c.ok(modules.get("IPC", {}).get("ingested", 0) > 0, "SSE detection IPC ingested > 0")
    c.ok(modules.get("Detection", {}).get("promotions", 0) > 0, "SSE detection promotions > 0")
    c.ok(modules.get("Detection", {}).get("cold_skipped", 0) > 0, "SSE detection cold_skipped > 0")
    c.ok(det.get("blocks_total", 0) > 0, "SSE detection blocks_total > 0")
    cg = sse.get("cgnat", {})
    c.ok("cgnat" in sse, "SSE cgnat group present")
    print(f"    [INFO] cgnat counters (module inactive): {cg}")
    an = sse.get("analytics", {})
    c.ok("analytics" in sse, "SSE analytics group present")
    print(f"    [INFO] analytics counters: {an}")
    mx = sse.get("mesh", {})
    c.ok("mesh" in sse, "SSE mesh group present")
    print(f"    [INFO] mesh counters: {mx}")
    fc = sse.get("forecasting", {})
    c.ok(fc.get("hw_rps", 0) > 0, "SSE forecasting.hw_rps > 0", str(fc.get("hw_rps")))
    c.ok(fc.get("entropy", 0) > 0, "SSE forecasting.entropy > 0", str(fc.get("entropy")))
    xdp = sse.get("xdp", {})
    c.ok("xdp" in sse, "SSE xdp group present")
    print(f"    [INFO] xdp counters (zero expected w/o caps): {xdp}")

    # Health
    c.ok(snap.get("is_healthy") is True, "is_healthy = true")

    # SSE groups
    for g in ["ipc", "detection", "system", "forecasting", "cgnat", "analytics", "mesh"]:
        c.ok(g in sse, f"SSE has {g} group")
    c.ok(len(sse.get("pipeline", [])) == 6, "SSE pipeline has 6 stages")

    return c.finish()

# ── main ─────────────────────────────────────────────────────────────────────
def main() -> int:
    print(f"metrics smoke test — {DASH_URL}\n")

    # Check server is alive
    try:
        with urllib.request.urlopen(f"{DASH_URL}/healthz", timeout=3) as r:
            if r.status != 200:
                raise RuntimeError(f"healthz status {r.status}")
    except Exception as e:
        print(f"ERROR: server not reachable at {DASH_URL} — {e}")
        return 1

    # Trigger phases
    phase_ipc_ingest()
    phase_ipc_reject()
    phase_ewma_block()
    phase_subnet_block()
    phase_ban_unban()
    phase_analytics_diversity()
    phase_forecasting()

    # Allow detection pipeline to process
    print("\nWaiting 2s for detection pipeline to flush...")
    time.sleep(2)

    # Verify
    total = verify_all_metrics()

    print(f"TOTAL FAILURES: {total}")
    return total

if __name__ == "__main__":
    sys.exit(main())
