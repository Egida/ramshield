#!/usr/bin/env python3
"""Trace coverage probe: drive every low-level TRACE arm, then prove it fired.

The Instrumentation Level Contract only holds if the trace arms are reachable.
This probe sends one stimulus per trace site through the IPC socket, then asserts
the matching family shows up in the newest runtime log.

Self-contained: stdlib only, no third-party deps.

Usage:
  python3 scripts/trace_coverage_probe.py                 # drive + verify
  python3 scripts/trace_coverage_probe.py --no-verify     # drive only
  python3 scripts/trace_coverage_probe.py --self-test     # frame schema check
"""

import argparse
import glob
import ipaddress
import json
import os
import socket
import sys
import time

HOST = "127.0.0.1"
PORT = 7890
LOG_DIR = os.environ.get("RAMSHIELD_LOG_DIR", "/tmp/ramshield-runtime")

# Canonical tokens only: an ad-hoc reason string makes the daemon log
# "unknown block reason" and pollutes the very audit this probe feeds.
BLOCK_REASON = "manual_block"

# Each entry: (trace family substring, human description of the stimulus).
EXPECTED = [
    ("ip cold-skipped", "low-cardinality unique subnets below the promote gate"),
    ("ip promoted to store", "hot source IP above the promote gate"),
    ("enforce applied", "IPC block/unblock round trip"),
    ("frame rejected", "deliberately malformed frame"),
]

# Arms that exist but cannot be driven from a client without absurd input.
# Listed so the gap is explicit rather than silently absent from the probe.
NOT_PROBED = [
    ("batch tail dropped", "BATCH_MAX is 1_000_000 events per frame"),
    ("event shed", "needs >=75% channel occupancy, not client-drivable"),
]


def ipc(frames, port=PORT):
    """Send frames on one persistent connection, collect the replies."""
    replies = []
    sock = socket.create_connection((HOST, port), timeout=10)
    sock.settimeout(10)
    try:
        for frame in frames:
            sock.sendall((json.dumps(frame) + chr(10)).encode())
            replies.append(json.loads(sock.recv(65536).decode().strip() or "{}"))
    finally:
        sock.close()
    return replies


def connection_events(ips, status=200, per_ip=1, proto_fp=7, size=512):
    events = []
    for ip in ips:
        for _ in range(per_ip):
            events.append({
                "ip": str(ip),
                "bytes": size,
                "status_code": status,
                "proto_fp": proto_fp,
            })
    return events


def stimulus_unique_subnets():
    """40 distinct /24s, one IP each -> below promote gate -> cold-skip trace."""
    ips = [ipaddress.ip_address("10.%d.0.%d" % (n, 10 + n)) for n in range(1, 41)]
    return [{"type": "report_connections",
             "events": connection_events(ips, per_ip=1)}]


def stimulus_hot_ip():
    """One IP with far more than promote_min_events -> promoted trace."""
    ip = ipaddress.ip_address("10.99.0.7")
    return [{"type": "report_connections",
             "events": connection_events([ip], per_ip=600, status=429, proto_fp=33)}]


def stimulus_block_roundtrip():
    """Enforcement traces: one block, one unblock (wire field is ttl_secs)."""
    ip = ipaddress.ip_address("10.99.0.8")
    return [
        {"type": "block_ip", "ip": str(ip), "reason": BLOCK_REASON, "ttl_secs": 60},
        {"type": "unblock_ip", "ip": str(ip)},
    ]


def stimulus_malformed():
    """Raw junk on the socket -> the parse-reject trace arm."""
    sock = socket.create_connection((HOST, PORT), timeout=10)
    try:
        sock.sendall(b'{"type": "not_a_real_request", "events": []}' + b"\n")
        sock.recv(65536)
    finally:
        sock.close()
    return []


def stimulus_oversized_batch():
    """Large but legal batch: exercises the same loop the tail-drop guard sits in."""
    ips = [ipaddress.ip_address("10.7.%d.%d" % (n // 250, n % 250)) for n in range(6000)]
    return [{"type": "report_connections", "events": connection_events(ips)}]


def drive():
    start = 0
    path = newest_log()
    if path:
        start = os.path.getsize(path)
    print("stimulus: cold-skip subnets")
    ipc(stimulus_unique_subnets())
    print("stimulus: hot IP promotion")
    ipc(stimulus_hot_ip())
    print("stimulus: block/unblock round trip")
    ipc(stimulus_block_roundtrip())
    print("stimulus: large batch")
    ipc(stimulus_oversized_batch())
    print("stimulus: malformed frame")
    stimulus_malformed()
    return start


def newest_log():
    logs = glob.glob(os.path.join(LOG_DIR, "*_trace.log")) + glob.glob(
        os.path.join(LOG_DIR, "*_runtime.log"))
    if not logs:
        return None
    return max(logs, key=os.path.getmtime)


def verify(start=0):
    path = newest_log()
    if not path:
        print("FAIL: no runtime log in " + LOG_DIR)
        return 1
    with open(path, "r", errors="replace") as handle:
        handle.seek(start)
        body = handle.read()
    missing = []
    for family, why in EXPECTED:
        count = body.count(family)
        status = "OK  " if count else "MISS"
        print("%s %-24s %5d  (%s)" % (status, family, count, why))
        if not count:
            missing.append(family)
    for family, why in NOT_PROBED:
        print("SKIP %-24s       (%s)" % (family, why))
    print("log: " + path)
    if missing:
        print("FAIL: trace arms never fired: " + ", ".join(missing))
        return 1
    print("PASS: every drivable low-level trace arm is reachable")
    return 0


def self_test():
    frames = (stimulus_unique_subnets() + stimulus_hot_ip()
              + stimulus_block_roundtrip() + stimulus_oversized_batch())
    for frame in frames:
        assert isinstance(frame, dict)
        assert "type" in frame
        if frame["type"] == "report_connections":
            assert frame["events"], "empty batch is not a stimulus"
            for event in frame["events"]:
                ipaddress.ip_address(event["ip"])
                assert isinstance(event["status_code"], int)
    assert stimulus_malformed() == []
    assert len(stimulus_oversized_batch()[0]["events"]) > 4096
    assert BLOCK_REASON in json.dumps(stimulus_block_roundtrip())
    assert len(EXPECTED) == 4
    print("OK: 4 stimuli, frames well-formed, canonical reason token only")
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--settle", type=float, default=8.0,
                        help="seconds to wait for the detection flush before verifying")
    parser.add_argument("--no-verify", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return self_test()
    start = drive()
    if args.no_verify:
        return 0
    time.sleep(args.settle)
    return verify(start)


if __name__ == "__main__":
    sys.exit(main())