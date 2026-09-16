#!/usr/bin/env python3
"""Full dashboard integration test with DDoS-scale event batches.

Runs all meaningful tests against a single server on scratch ports (17890/19999).
Uses low detection thresholds via env overrides so EWMA auto-block and subnet
blocking fire within the test's time budget.

Sequence:
  1. lint    — cargo fmt --check + clippy -D warnings
  2. unit    — cargo test --all (Rust unit + integration)
  3. boot    — start release binary with tuned detection env overrides
  4. e2e     — protocol + detection (EWMA auto-block, subnet block, invalid frames)
  5. burst   — attack_nexus.py l7_http_flood 15s (128 workers × 1500 batch)
  6. capture — live_capture.py SSE + load simultaneous (20s)
  7. verify  — dashboard snapshot under post-load conditions
  8. teardown

Exit code = total failed steps.
"""
from __future__ import annotations

import argparse
import json
import os
import signal
import socket
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
BIN = REPO / "target" / "release" / "ramshield"
IPC_PORT = 17890
DASH_PORT = 19999
IPC_ADDR = f"127.0.0.1:{IPC_PORT}"
DASH_URL = f"http://127.0.0.1:{DASH_PORT}"
START_TIMEOUT = 30.0

# Detection-tuned env overrides — low thresholds so auto-block fires in <5s.
TUNED_ENV = dict(
    os.environ,
    RAMSHIELD_IPC__TCP_ADDR=IPC_ADDR,
    RAMSHIELD_DASHBOARD__HTTP_ADDR=f"127.0.0.1:{DASH_PORT}",
    RAMSHIELD_DASHBOARD__ENABLED="true",
    RAMSHIELD_DETECTION__RPS_THRESHOLD="50",
    RAMSHIELD_DETECTION__RATE_WINDOW_SECS="1",
    RAMSHIELD_DETECTION__PROMOTE_MIN_EVENTS="2",
    RAMSHIELD_DETECTION__SUBNET_BATCH_THRESHOLD="1",
    RAMSHIELD_DETECTION__SUBNET_BATCH_MIN_EVENTS="1",
    RAMSHIELD_DETECTION__BATCH_BLOCK_ENABLED="true",
)


def sh(*args: str, cwd: Path = REPO, timeout: int | None = None, env: dict | None = None) -> int:
    print(f"  $ {' '.join(args)}")
    return subprocess.run(args, cwd=cwd, timeout=timeout, env=env).returncode


def sh_out(*args: str, cwd: Path = REPO, timeout: int | None = None, env: dict | None = None) -> tuple[int, str]:
    r = subprocess.run(args, cwd=cwd, capture_output=True, text=True, timeout=timeout, env=env)
    return r.returncode, r.stdout + r.stderr


class Check:
    def __init__(self, name: str) -> None:
        self.name = name
        self.passed = 0
        self.failed: list[str] = []

    def ok(self, cond: bool, desc: str, detail: str = "") -> bool:
        mark = "PASS" if cond else "FAIL"
        print(f"    [{mark}] {desc}" + (f" — {detail}" if detail and not cond else ""))
        if cond:
            self.passed += 1
        else:
            self.failed.append(desc)
        return cond

    def finish(self) -> int:
        total = self.passed + len(self.failed)
        status = "OK" if not self.failed else f"FAILED {len(self.failed)}/{total}"
        print(f"  == {self.name}: {status} ({self.passed}/{total} passed)\n")
        return len(self.failed)


class Server:
    def __init__(self) -> None:
        self.proc: subprocess.Popen | None = None

    def __enter__(self) -> "Server":
        if not BIN.exists():
            raise SystemExit(f"release binary missing: {BIN}\n  cargo build --release -F full")
        self.log = open("/tmp/ramshield_suite_server.log", "a")
        self.proc = subprocess.Popen(
            [str(BIN), "--config", "config-xdp.toml"],
            cwd=str(REPO), env=TUNED_ENV,
            stdout=self.log, stderr=subprocess.STDOUT,
            start_new_session=True,
        )
        end = time.monotonic() + START_TIMEOUT
        while time.monotonic() < end:
            try:
                with urllib.request.urlopen(f"{DASH_URL}/healthz", timeout=1) as r:
                    if r.status == 200:
                        break
            except Exception:
                time.sleep(0.25)
        else:
            self.__exit__(None, None, None)
            raise SystemExit("server failed to become healthy")
        if not self._port_owned_by(IPC_PORT, self.proc.pid):
            time.sleep(0.5)
        if not self._port_owned_by(IPC_PORT, self.proc.pid):
            self.__exit__(None, None, None)
            raise SystemExit(
                f"suite server did not bind {IPC_ADDR} (stray process or bind failure); "
                f"see /tmp/ramshield_suite_server.log"
            )
        print(f"  server up: ipc={IPC_ADDR} dash={DASH_URL} (pid {self.proc.pid})")
        return self

    @staticmethod
    def _port_owned_by(port: int, pid: int) -> bool:
        """True when <pid> owns the LISTEN socket on <port>.
        /proc-based: ss -p shows no PID column for other users on this host."""
        try:
            want = f"{port:04X}"
            inode = None
            for table in ("/proc/net/tcp", "/proc/net/tcp6"):
                with open(table) as f:
                    next(f)
                    for line in f:
                        p = line.split()
                        if len(p) >= 10 and p[3] == "0A" and p[1].split(":")[1] == want:
                            inode = p[9]
                            break
                if inode:
                    break
            if not inode:
                return False
            fd_dir = f"/proc/{pid}/fd"
            for fd in os.listdir(fd_dir):
                try:
                    if os.readlink(os.path.join(fd_dir, fd)).endswith(f"socket:[{inode}]"):
                        return True
                except OSError:
                    continue
            return False
        except Exception:
            return True  # /proc unavailable → skip ownership check

    def __exit__(self, *exc) -> None:
        if self.proc:
            try:
                os.killpg(self.proc.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                self.proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(self.proc.pid, signal.SIGKILL)
                self.proc.wait(timeout=3)
        print("  server stopped")


def ipc(payload: dict, timeout: float = 5.0) -> dict:
    with socket.create_connection(("127.0.0.1", IPC_PORT), timeout=timeout) as s:
        s.sendall((json.dumps(payload) + "\n").encode())
        s.settimeout(timeout)
        buf = b""
        while b"\n" not in buf:
            chunk = s.recv(65536)
            if not chunk:
                break
            buf += chunk
        return json.loads(buf.split(b"\n")[0])


def fetch_sse_frame(timeout=2):
    """Read one SSE frame from /api/stream using Python HTTP (avoids curl hang on streaming)."""
    try:
        req = urllib.request.Request(f"{DASH_URL}/api/stream")
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


# ── layers ────────────────────────────────────────────────────────────────────

def layer_lint() -> int:
    c = Check("lint")
    c.ok(sh("cargo", "fmt", "--all", "--check", timeout=120) == 0, "cargo fmt --check")
    c.ok(sh("cargo", "clippy", "--all-targets", "--", "-D", "warnings", timeout=600) == 0,
         "cargo clippy -D warnings")
    return c.finish()


def layer_unit() -> int:
    c = Check("unit")
    rc, out = sh_out("cargo", "test", "--all", timeout=900)
    c.ok(rc == 0, "cargo test --all")
    if rc != 0:
        print(out[-2000:])
    return c.finish()


def layer_e2e(srv: Server) -> int:
    c = Check("e2e-protocol")

    # health
    with urllib.request.urlopen(f"{DASH_URL}/healthz", timeout=3) as r:
        body = json.load(r)
    c.ok(body.get("status") == "ok", "healthz returns {status: ok}", str(body))

    # check unknown IP
    r = ipc({"type": "check_ip", "ip": "203.0.113.7"})
    c.ok(r.get("type") == "ip_status" and not r.get("blocked"),
         "check_ip unknown → clean", str(r))

    # block/unblock round-trip
    r = ipc({"type": "block_ip", "ip": "203.0.113.7", "reason": "suite", "ttl_secs": 120})
    c.ok(r.get("type") == "ok", "block_ip accepted")
    r = ipc({"type": "check_ip", "ip": "203.0.113.7"})
    c.ok(bool(r.get("blocked")), "check_ip blocked after block_ip")
    r = ipc({"type": "unblock_ip", "ip": "203.0.113.7"})
    c.ok("error" not in r, "unblock_ip accepted")
    r = ipc({"type": "check_ip", "ip": "203.0.113.7"})
    c.ok(not r.get("blocked"), "check_ip clean after unblock_ip")

    # EWMA auto-block: 200 events per batch, 0.05s sleep.
    # With rps_threshold=50, rate_window_secs=1, pulse_threshold=2:
    #   each batch = 200 eps; detection flushes every 1s.
    #   After 2 consecutive 1s windows > 50, block fires.
    blocked = False
    for round_ in range(120):
        ev = [{"ip": "198.51.100.66", "bytes": 512, "status_code": 200, "proto_fp": 0x1000}
              for _ in range(200)]
        ipc({"type": "report_connections", "events": ev})
        if round_ % 5 == 0:
            time.sleep(0.05)
        if ipc({"type": "check_ip", "ip": "198.51.100.66"}).get("blocked"):
            blocked = True
            break
        time.sleep(0.02)
    c.ok(blocked, "EWMA auto-block fires (threshold=50, window=1s)")

    # subnet block: 250 distinct IPs from /24, 3 rounds
    events = [{"ip": f"192.0.2.{i}", "bytes": 256, "status_code": 404, "proto_fp": 0x1000}
              for i in range(250)]
    for _ in range(5):
        ipc({"type": "report_connections", "events": events})
        time.sleep(0.1)
    subnet_blocked = False
    for _ in range(60):
        time.sleep(0.5)
        if ipc({"type": "check_ip", "ip": "192.0.2.199"}).get("blocked"):
            subnet_blocked = True
            break
    c.ok(subnet_blocked, "subnet /24 block fires (threshold=3 rounds)")

    # dashboard snapshot healthy + pipeline counts
    with urllib.request.urlopen(f"{DASH_URL}/api/snapshot", timeout=3) as resp:
        snap = json.load(resp)
    c.ok(snap.get("is_healthy") is True, "dashboard snapshot healthy")
    pipeline = snap.get("pipeline", {})
    c.ok(pipeline.get("blocked", 0) > 0, "snapshot pipeline shows blocked > 0", str(pipeline))
    c.ok(snap.get("ips_tracked", 0) > 0, "snapshot ips_tracked > 0", str(snap.get("ips_tracked")))

    # SSE stream sanity: top-level groups present
    d = fetch_sse_frame()
    if d:
        c.ok("ipc" in d, "SSE has ipc group")
        c.ok("detection" in d, "SSE has detection group")
        c.ok("system" in d, "SSE has system group")
        c.ok("pipeline" in d and len(d.get("pipeline", [])) == 6, "SSE has 6-stage pipeline")
    else:
        c.ok(False, "SSE frame received")

    # invalid IP → typed error frame, connection survives
    r = ipc({"type": "check_ip", "ip": "not-an-ip"})
    c.ok(r.get("type") == "error" and r.get("code") == 400, "invalid IP → 400 error frame")
    r = ipc({"type": "check_ip", "ip": "203.0.113.9"})
    c.ok(r.get("type") == "ip_status", "connection alive after bad frame")

    return c.finish()


def layer_nexus_burst(srv: Server) -> int:
    """attack_nexus.py: 15s flood with 128 workers on the shared server."""
    c = Check("nexus-burst-15s")
    rc = sh(sys.executable, str(REPO / "scripts" / "attack_nexus.py"),
            "--port", str(IPC_PORT),
            "run", "--profile", "l7_http_flood", "--duration", "15",
            timeout=120)
    c.ok(rc == 0, "attack_nexus burst completed", f"rc={rc}")

    # post-burst snapshot
    try:
        with urllib.request.urlopen(f"{DASH_URL}/api/snapshot", timeout=3) as resp:
            snap = json.load(resp)
        c.ok(snap.get("is_healthy") is True, "dashboard healthy post-burst")
        blocked = snap.get("pipeline", {}).get("blocked", 0)
        c.ok(blocked > 0, f"snapshot blocked={blocked} > 0 post-burst")
        tracked = snap.get("ips_tracked", 0)
        c.ok(tracked > 100, f"snapshot ips_tracked={tracked} > 100 post-burst")

        # SSE post-burst: all groups present, counts non-zero
        d = fetch_sse_frame()
        if d:
            ipc_ingest = d.get("ipc", {}).get("ingest_total", 0)
            c.ok(ipc_ingest > 100_000, f"SSE ingest_total={ipc_ingest:,} > 100k")
            det_blocks = 0
            for m in d.get("detection", {}).get("modules", []):
                if m.get("label") == "Detection":
                    det_blocks = m.get("detail", {}).get("blocks", 0)
                    break
            c.ok(det_blocks > 0, f"SSE detection blocks={det_blocks} > 0")
        else:
            c.ok(False, "SSE frame received post-burst")
    except Exception as e:
        c.ok(False, "post-burst verification", str(e))

    return c.finish()


def layer_live_capture(srv: Server) -> int:
    """live_capture.py: simultaneous load + SSE capture."""
    c = Check("live-capture")
    rc = sh("python3", str(REPO / "scripts" / "live_capture.py"), timeout=60,
            env=dict(os.environ, RAMSHIELD_TEST_HOST="127.0.0.1",
                     RAMSHIELD_TEST_IPC_PORT=str(IPC_PORT),
                     RAMSHIELD_TEST_DASH_URL=DASH_URL))
    c.ok(rc == 0, "live_capture completed", f"rc={rc}")

    # verify SSE still streams after live_capture traffic
    d = fetch_sse_frame()
    if d:
        c.ok(d.get("health", {}).get("is_healthy") is True, "SSE health still ok post-capture")
        c.ok(len(d.get("pipeline", [])) == 6, "SSE pipeline still 6 stages post-capture")
    else:
        c.ok(False, "SSE frame received post-capture")
    return c.finish()


def layer_final_verify(srv: Server) -> int:
    """Final dashboard verification under accumulated load."""
    c = Check("final-verify")

    # snapshot
    with urllib.request.urlopen(f"{DASH_URL}/api/snapshot", timeout=3) as resp:
        snap = json.load(resp)
    c.ok(snap.get("is_healthy") is True, "final snapshot healthy")
    c.ok(snap.get("ipc_requests", 0) > 0, f"ipc_requests={snap.get('ipc_requests')} > 0")
    c.ok(snap.get("events_ingested", 0) > 10_000, f"events_ingested={snap.get('events_ingested',0):,} > 10k")
    c.ok(snap.get("pipeline", {}).get("blocked", 0) > 0, "pipeline blocked > 0 final")

    # API endpoints respond
    for path in ["/healthz", "/metrics", "/api/snapshot", "/api/blocks/active",
                 "/api/history/batches", "/api/history/blocks", "/api/status/modules"]:
        try:
            with urllib.request.urlopen(f"{DASH_URL}{path}", timeout=3) as r:
                c.ok(r.status == 200, f"{path} returns 200")
        except Exception as e:
            c.ok(False, f"{path} returns 200", str(e))

    # SSE stream: 6 top-level groups
    d = fetch_sse_frame()
    if d:
        groups = ["ipc", "detection", "system", "forecasting", "cgnat", "analytics", "mesh"]
        for g in groups:
            c.ok(g in d, f"SSE has {g} group")
        c.ok(len(d.get("pipeline", [])) == 6, "SSE pipeline has 6 stages")
    else:
        c.ok(False, "SSE frame received")

    return c.finish()


# ── CLI ───────────────────────────────────────────────────────────────────────

def main() -> int:
    ap = argparse.ArgumentParser(description="Full dashboard integration test")
    ap.add_argument("--skip-unit", action="store_true", help="skip cargo test")
    ap.add_argument("--skip-nexus", action="store_true", help="skip attack_nexus burst")
    args = ap.parse_args()

    print(f"dashboard integration test — repo {REPO}\n")
    total = 0

    # Phase 1: lint + unit (no server needed)
    total += layer_lint()
    if not args.skip_unit:
        total += layer_unit()

    # Phase 2: server lifecycle — all subsequent layers share one server
    with Server() as srv:
        total += layer_e2e(srv)
        if not args.skip_nexus:
            total += layer_nexus_burst(srv)
        total += layer_live_capture(srv)
        total += layer_final_verify(srv)

    print(f"TOTAL FAILURES: {total}")
    return total


if __name__ == "__main__":
    sys.exit(main())