#!/usr/bin/env python3
"""Aggressive scenario battery with per-module delta monitoring.

Boots ONE isolated daemon on scratch ports (IPC 27890 / dashboard 29999),
runs each attack_nexus profile against it, and prints per-module deltas so
every counter change is attributable to a named scenario.

Never touches a live instance: production binds are 7890/9999, suite.py
already occupies 17890/19999.

Usage:
  python3 scripts/scenario_battery.py                      # default set
  python3 scripts/scenario_battery.py --duration 45
  python3 scripts/scenario_battery.py --profiles l7_http_flood botnet_entropy
"""

from __future__ import annotations

import argparse
import json
import os
import signal
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
BIN = REPO / "target" / "release" / "ramshield"
IPC_PORT = 27890
DASH_PORT = 29999
IPC_ADDR = f"127.0.0.1:{IPC_PORT}"
DASH_URL = f"http://127.0.0.1:{DASH_PORT}"
START_TIMEOUT = 30.0

# Test-scale detection thresholds: production defaults (rps 1000) never fire
# on a 30-60 s scenario. Mirrors ramshield-development skill guidance.
DETECTION_ENV = {
    "RAMSHIELD_DETECTION__RPS_THRESHOLD": "50",
    "RAMSHIELD_DETECTION__RATE_WINDOW_SECS": "1",
    "RAMSHIELD_DETECTION__PROMOTE_MIN_EVENTS": "2",
    "RAMSHIELD_DETECTION__SUBNET_BATCH_THRESHOLD": "3",
    "RAMSHIELD_DETECTION__SUBNET_BATCH_MIN_EVENTS": "10",
    "RAMSHIELD_DETECTION__BATCH_BLOCK_ENABLED": "true",
}

DEFAULT_PROFILES = [
    "l7_http_flood",
    "volumetric_syn",
    "slowloris",
    "dns_amplification",
    "botnet_entropy",
    "api_abuse",
    "red_team_full",
    "subnet_ddos_5min",
]

# Chain profiles run fixed per-phase duration; --duration is ignored for them.
CHAIN_PROFILES = {"red_team_full"}

# metric key paths, flattened for delta reporting
SNAP_KEYS = [
    "events_ingested", "events_rejected", "frames_rejected_total",
    "events_shed", "channel_depth", "batches_total", "promotions",
    "cold_skipped", "blocks_applied", "ips_tracked", "ram_bytes",
    "cpu_usage", "memory_usage_mb", "xdp_active", "wal_lsn",
]

MODULE_KEYS = {
    "IPC": ["ingested", "rejected"],
    "Detection": ["batches", "blocks", "promotions", "subnet_blocks",
                  "cold_skipped", "last_batch_events"],
    "Forecasting": ["forecast_ticks", "forecast_blocks", "entropy_ticks"],
    "CGNAT": ["classify_ticks", "shm_publishes", "shm_cache_hits",
              "tier_allow", "tier_block", "tier_challenge", "tier_powdrop"],
    "Analytics": ["cms_increments", "hll_inserts"],
    "Mesh": ["record_bans", "record_unbans", "hlc_ticks", "purge_ticks"],
    "Storage": ["ips_tracked"],
}


def get_json(url: str, timeout: float = 5.0):
    with urllib.request.urlopen(url, timeout=timeout) as r:
        return json.load(r)


def wait_ready(deadline: float = START_TIMEOUT) -> bool:
    end = time.monotonic() + deadline
    while time.monotonic() < end:
        try:
            if get_json(f"{DASH_URL}/healthz", 1).get("status") == "ok":
                return True
        except Exception:
            time.sleep(0.25)
    return False


def snapshot():
    return get_json(f"{DASH_URL}/api/snapshot")


def modules():
    return {m["label"]: m for m in get_json(f"{DASH_URL}/api/status/modules")}


def flat_mod(mods):
    out = {}
    for label, m in mods.items():
        for k in MODULE_KEYS.get(label, []):
            out[f"{label}.{k}"] = m.get("detail", {}).get(k, 0)
    return out


def owner_of_port(port: int) -> str:
    """Prove which PID owns a listening port via /proc/net/tcp inode."""
    hexport = f"{port:04X}"
    inode = None
    for line in Path("/proc/net/tcp").read_text().splitlines()[1:]:
        f = line.split()
        if f[1].endswith(f":{hexport}") and f[3] == "0A":
            inode = f[9]
            break
    if not inode:
        return "?"
    for p in Path("/proc").iterdir():
        if not p.name.isdigit():
            continue
        try:
            for fd in (p / "fd").iterdir():
                if os.readlink(fd) == f"socket:[{inode}]":
                    return p.name
        except (PermissionError, FileNotFoundError, ProcessLookupError):
            continue
    return "?"


def delta(before: dict, after: dict) -> dict:
    return {k: after.get(k, 0) - before.get(k, 0)
            for k in after if isinstance(after.get(k), (int, float))
            and isinstance(before.get(k), (int, float))}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--duration", type=float, default=30.0)
    ap.add_argument("--profiles", nargs="*", default=DEFAULT_PROFILES)
    ap.add_argument("--drain", type=float, default=3.0)
    ap.add_argument("--workers", type=int, default=None,
                    help="override profile worker count; omit to keep profile defaults")
    args = ap.parse_args()

    if not BIN.exists():
        print(f"release binary missing: {BIN}", file=sys.stderr)
        return 2

    env = dict(os.environ, **DETECTION_ENV,
               RAMSHIELD_IPC__TCP_ADDR=IPC_ADDR,
               RAMSHIELD_DASHBOARD__HTTP_ADDR=f"127.0.0.1:{DASH_PORT}",
               RAMSHIELD_DASHBOARD__ENABLED="true")

    print(f"scratch daemon: ipc={IPC_ADDR} dash={DASH_URL}")
    proc = subprocess.Popen(
        [str(BIN), "--config", "config.toml"], cwd=str(REPO), env=env,
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        start_new_session=True,
    )
    try:
        if not wait_ready():
            print("FAIL: daemon never became healthy", file=sys.stderr)
            return 3
        pid = owner_of_port(IPC_PORT)
        print(f"server pid={proc.pid}  ipc:{IPC_PORT} owned by pid={pid} "
              f"{'OK' if pid == str(proc.pid) else 'MISMATCH — aborting'}")
        if pid != str(proc.pid):
            return 4

        base_s = snapshot()
        base_m = flat_mod(modules())
        print(f"baseline: ingested={base_s['events_ingested']} "
              f"xdp_active={base_s['xdp_active']} wal_lsn={base_s['wal_lsn']}\n")

        results = []
        for prof in args.profiles:
            print(f"── {prof} ({args.duration:.0f}s) " + "─" * 30)
            t0 = time.monotonic()
            # --port/--workers are TOP-LEVEL (before the `run` subcommand).
            # Chain profiles ignore --duration (fixed per-phase secs); allow
            # chain phases to overrun the per-scenario budget.
            is_chain = prof in CHAIN_PROFILES
            cmd = [sys.executable, "scripts/attack_nexus.py",
                   "--port", str(IPC_PORT)]
            if args.workers is not None:
                cmd += ["--workers", str(args.workers)]
            cmd += ["run", "--profile", prof, "--duration", str(args.duration)]
            rc = subprocess.run(
                cmd,
                cwd=str(REPO),
                timeout=(args.duration * 8 + 180) if is_chain
                        else (args.duration + 120),
            ).returncode
            elapsed = time.monotonic() - t0
            time.sleep(args.drain)

            s = snapshot()
            f = flat_mod(modules())
            ds, dm = delta(base_s, s), delta(base_m, f)

            ingested = ds.get("events_ingested", 0)
            eps = ingested / elapsed if elapsed else 0
            print(f"  exit={rc} elapsed={elapsed:.1f}s "
                  f"ingested=+{ingested} ({eps:,.0f} ev/s) "
                  f"rejected=+{ds.get('events_rejected', 0)} "
                  f"shed=+{ds.get('events_shed', 0)} "
                  f"chan_depth={s.get('channel_depth')}")
            for group in ("Detection", "Forecasting", "CGNAT", "Analytics",
                          "Mesh", "Storage"):
                changed = {k.split(".", 1)[1]: v for k, v in dm.items()
                           if k.startswith(group + ".") and v}
                if changed:
                    print(f"  {group:<12} " +
                          "  ".join(f"{k}=+{v}" for k, v in changed.items()))
            results.append((prof, rc, ingested, dm))
            base_s, base_m = s, f

        print("\n" + "=" * 68)
        print(f"{'profile':<20}{'exit':>5}{'ingested':>12}{'blocks':>10}"
              f"{'subnets':>10}{'forecast':>10}")
        for prof, rc, ing, dm in results:
            print(f"{prof:<20}{rc:>5}{ing:>12,}"
                  f"{dm.get('Detection.blocks', 0):>10}"
                  f"{dm.get('Detection.subnet_blocks', 0):>10}"
                  f"{dm.get('Forecasting.forecast_blocks', 0):>10}")

        bad = [p for p, rc, ing, _ in results if rc != 0 or ing == 0]
        print(f"\n{'FAIL — no-ingest or nonzero exit: ' + ', '.join(bad) if bad else 'ALL SCENARIOS INGESTED'}")
        return 1 if bad else 0
    finally:
        try:
            os.killpg(proc.pid, signal.SIGTERM)
            proc.wait(timeout=5)
        except (ProcessLookupError, subprocess.TimeoutExpired):
            try:
                os.killpg(proc.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        print("server stopped")


if __name__ == "__main__":
    sys.exit(main())